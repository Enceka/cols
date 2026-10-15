//! Offline selection of captured TDM starts using the gameplay spawn capsule.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

#[derive(Serialize)]
struct Rejected {
    team: usize,
    index: usize,
    origin: [f32; 3],
    error: String,
}

#[derive(Serialize)]
struct Selection {
    original_counts: [usize; 2],
    retained: [Vec<[f32; 3]>; 2],
    rejected: Vec<Rejected>,
}

impl Selection {
    fn enough(&self, minimum: usize) -> bool {
        self.retained
            .iter()
            .all(|team| (minimum..=128).contains(&team.len()))
    }
    fn headings(&self, headings: [Vec<[u16; 3]>; 2]) -> Result<[Vec<[u16; 3]>; 2]> {
        ensure!(
            headings.each_ref().map(|a| a.len()) == self.original_counts,
            "spawn headings do not match captured coordinates"
        );
        Ok(std::array::from_fn(|team| {
            headings[team]
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    !self
                        .rejected
                        .iter()
                        .any(|r| r.team == team + 1 && r.index == *index)
                })
                .map(|(_, angle)| *angle)
                .collect()
        }))
    }
}

fn select(
    starts: [Vec<[f32; 3]>; 2],
    collision: &crate::game::world::collision::Collision,
) -> Selection {
    let mut result = Selection {
        original_counts: starts.each_ref().map(|team| team.len()),
        retained: Default::default(),
        rejected: Vec::new(),
    };
    for (team, starts) in starts.into_iter().enumerate() {
        for (index, origin) in starts.into_iter().enumerate() {
            match crate::game::ai::controller::grounded_spawn(origin, collision) {
                Ok(_) => result.retained[team].push(origin),
                Err(error) => result.rejected.push(Rejected {
                    team: team + 1,
                    index,
                    origin,
                    error: error.to_string(),
                }),
            }
        }
    }
    result
}

fn write_new(path: &Path, json: &serde_json::Value) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(json)?)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn prepare(source: &Path, output: &Path, report: &Path) -> Result<()> {
    ensure!(
        !output.exists() && !report.exists() && output != report,
        "spawn selection outputs must be new"
    );
    ensure!(
        std::fs::metadata(source)?.len() <= 8 * 1024 * 1024,
        "map preparation exceeds 8 MiB"
    );
    let bytes = std::fs::read(source)?;
    let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
    let map = json["map"]
        .as_str()
        .context("map identity missing")?
        .to_owned();
    let crc = json["ncs_crc"]
        .as_u64()
        .filter(|n| *n <= u64::from(u32::MAX))
        .context("invalid map CRC")? as u32;
    ensure!(
        json["schema_version"] == 1
            && json["client_sha256"] == codol_protocol::CLIENT_SHA256
            && json["mode"]
                .as_str()
                .is_some_and(|mode| crate::content::maps::registry::supports_mode(&map, mode))
            && json["observed_crc_verified"] == true
            && crate::content::maps::registry::native_id(&map).is_some(),
        "spawn selection requires an initialized native TDM preparation"
    );
    let parent = source.parent().unwrap_or(Path::new("."));
    for field in [
        "collision_file",
        "arsenal_file",
        "appearance_file",
        "voice_file",
        "nuke_file",
        "radar_file",
        "props_file",
    ] {
        if let Some(file) = json[field].as_str() {
            json[field] = serde_json::Value::String(
                parent.join(file).canonicalize()?.to_string_lossy().into(),
            );
        }
    }
    let collision = crate::game::world::collision::Collision::load(Path::new(
        json["collision_file"]
            .as_str()
            .context("map collision missing")?,
    ))?;
    ensure!(
        collision.map == format!("maps/mp/{map}.d3dbsp") && collision.ncs_crc == crc,
        "spawn collision map or CRC mismatch"
    );
    let starts: [Vec<[f32; 3]>; 2] =
        serde_json::from_value(json["match_config"]["spawns"].clone())?;
    ensure!(
        starts.iter().all(|team| team.len() <= 128),
        "captured spawn count exceeds limit"
    );
    let selection = select(starts, &collision);
    let points: Vec<crate::game::modes::tdm::SpawnPoint> = json["match_config"]
        .get("respawn_points")
        .map(|p| serde_json::from_value(p.clone()))
        .transpose()?
        .unwrap_or_default();
    ensure!(points.len() <= 1024, "shared respawn count exceeds limit");
    let mut retained = Vec::new();
    let mut rejected = Vec::new();
    for (index, point) in points.iter().enumerate() {
        match crate::game::ai::controller::grounded_spawn(point.origin, &collision) {
            Ok(_) => {
                retained.push(serde_json::json!({"origin":point.origin,"angles":point.angles}))
            }
            Err(error) => rejected.push(
                serde_json::json!({"index":index,"origin":point.origin,"error":error.to_string()}),
            ),
        }
    }
    json["match_config"]["respawn_points"] = serde_json::json!(retained);
    let mut evidence = serde_json::json!({"schema_version":1,"native_client_used":false,"map":map,
        "ncs_crc":crc,"source_sha256":format!("{:x}",Sha256::digest(&bytes)),
        "selection":selection,"prepared":false});
    evidence["respawn_selection"] = serde_json::json!({"original_count":points.len(),"retained_count":retained.len(),"rejected":rejected});
    if !points.is_empty() && retained.is_empty() {
        write_new(report, &evidence)?;
        anyhow::bail!(
            "no clear captured shared respawn point; see {}",
            report.display()
        );
    }
    if !json["match_config"]["base_respawns"].is_null() {
        let bases: [Vec<crate::game::modes::tdm::SpawnPoint>; 2] =
            serde_json::from_value(json["match_config"]["base_respawns"].clone())?;
        let mut selected = [Vec::new(), Vec::new()];
        let mut refused = Vec::new();
        for (team, base) in bases.iter().enumerate() {
            ensure!(base.len() <= 128, "fixed respawn pool too large");
            for (index, point) in base.iter().enumerate() {
                match crate::game::ai::controller::grounded_spawn(point.origin,&collision) {
                    Ok(_) => selected[team].push(serde_json::json!({"origin":point.origin,"angles":point.angles})),
                    Err(error)=>refused.push(serde_json::json!({"team":team+1,"index":index,"origin":point.origin,"error":error.to_string()}))
                }
            }
        }
        evidence["base_respawn_selection"] = serde_json::json!({"original_counts":bases.each_ref().map(Vec::len),"retained_counts":selected.each_ref().map(Vec::len),"rejected":refused});
        if selected.iter().any(Vec::is_empty) {
            write_new(report, &evidence)?;
            anyhow::bail!("no clear fixed-base respawn; see {}", report.display());
        }
        json["match_config"]["base_respawns"] = serde_json::json!(selected);
    }
    let players = json["match_config"]["player_count"].as_u64().unwrap_or(12);
    ensure!(matches!(players, 4 | 12), "invalid prepared match size");
    if !selection.enough(players as usize / 2) {
        write_new(report, &evidence)?;
        anyhow::bail!(
            "too few clear captured starts for the active roster; see {}",
            report.display()
        );
    }
    json["match_config"]["spawns"] = serde_json::to_value(&selection.retained)?;
    if !json["match_config"]["spawn_angles"].is_null() {
        let headings = serde_json::from_value(json["match_config"]["spawn_angles"].clone())?;
        json["match_config"]["spawn_angles"] = serde_json::to_value(selection.headings(headings)?)?;
    }
    json["spawn_origin"] = serde_json::to_value(selection.retained[1][0])?;
    if json["training_enemy"].is_object() {
        json["training_enemy"]["origin"] = serde_json::to_value(selection.retained[0][0])?;
    }
    write_new(output, &json)?;
    let checked = crate::game::session::prepared::PreparedGameState::load(output);
    evidence["prepared"] = serde_json::Value::Bool(checked.is_ok());
    if let Err(error) = &checked {
        evidence["validation_error"] = serde_json::Value::String(format!("{error:#}"));
    }
    write_new(report, &evidence)?;
    checked.context("validate map after selecting captured starts")?;
    Ok(())
}

#[cfg(test)]
mod heading_tests {
    #[test]
    fn rejected_starts_remove_their_own_headings_without_changing_team_order() {
        let selection = super::Selection {
            original_counts: [3, 2],
            retained: Default::default(),
            rejected: vec![super::Rejected {
                team: 1,
                index: 1,
                origin: [0.0; 3],
                error: "blocked".into(),
            }],
        };
        assert_eq!(
            selection
                .headings([
                    vec![[0, 10, 0], [0, 20, 0], [0, 30, 0]],
                    vec![[0, 40, 0], [0, 50, 0]]
                ])
                .unwrap(),
            [vec![[0, 10, 0], [0, 30, 0]], vec![[0, 40, 0], [0, 50, 0]]]
        );
        assert!(selection.headings([vec![[0; 3]], vec![[0; 3]; 2]]).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> crate::game::world::collision::Collision {
        let data = serde_json::json!({"schema_version":1,"read_only":true,
            "client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,
            "brushes":[{"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        crate::game::world::collision::Collision::load(file.path()).unwrap()
    }

    #[test]
    fn blocked_capture_is_excluded_without_inventing_or_repositioning_starts() {
        let valid: Vec<_> = (0..6).map(|i| [i as f32 * 100.0, 0.0, 8.0]).collect();
        let mut axis = valid.clone();
        axis.insert(2, [0.0, 0.0, -1000.0]);
        let result = select([axis, valid.clone()], &world());
        assert!(result.enough(6));
        assert_eq!(result.retained, [valid.clone(), valid]);
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].team, 1);
        assert_eq!(result.rejected[0].index, 2);
        assert!(!result.rejected[0].error.is_empty());
    }

    #[test]
    fn fewer_than_six_clear_points_stays_unprepared() {
        let valid: Vec<_> = (0..6).map(|i| [i as f32 * 100.0, 0.0, 8.0]).collect();
        let result = select([valid[..5].to_vec(), valid], &world());
        assert!(!result.enough(6));
    }
}
