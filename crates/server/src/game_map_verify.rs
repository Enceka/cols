//! Fast server validation over real prepared resources; no native visual claim.
use crate::{game_map_catalog::GameMapCatalog, game_state::PreparedGameState};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub client_sha256: &'static str,
    pub native_client_used: bool,
    pub catalog_load_msec: u128,
    pub simulation_elapsed_msec: u128,
    pub elapsed_msec: u128,
    pub simulation_msec: u32,
    pub passed: bool,
    pub maps: Vec<MapReport>,
}
#[derive(Serialize)]
pub struct MapReport {
    pub map: String,
    pub ncs_crc: u32,
    pub validation_seed: u64,
    pub passed: bool,
    pub elapsed_msec: u128,
    pub moved_bots: usize,
    pub destroyed_glass: usize,
    pub kills: usize,
    pub respawns: usize,
    pub top_players_scene: bool,
    pub errors: Vec<String>,
}
pub fn verify(catalog: &GameMapCatalog, simulation_msec: u32) -> Result<Report> {
    ensure!(
        (15000..=120000).contains(&simulation_msec),
        "validation duration must be 15..120 seconds"
    );
    let start = std::time::Instant::now();
    let mut maps = Vec::new();
    for state in catalog.preparations() {
        let started = std::time::Instant::now();
        let mut row = MapReport {
            map: state.map().into(),
            ncs_crc: state.ncs_crc(),
            validation_seed: 0x1234_5678 ^ u64::from(state.ncs_crc()),
            passed: false,
            elapsed_msec: 0,
            moved_bots: 0,
            destroyed_glass: 0,
            kills: 0,
            respawns: 0,
            top_players_scene: false,
            errors: Vec::new(),
        };
        if let Err(error) = simulate(state, simulation_msec, &mut row) {
            row.errors.push(format!("{error:#}"));
        }
        row.passed = row.errors.is_empty();
        row.elapsed_msec = started.elapsed().as_millis();
        tracing::info!(map=%row.map,passed=row.passed,msec=row.elapsed_msec,moved=row.moved_bots,kills=row.kills,
            "prepared map validation");
        maps.push(row);
    }
    ensure!(!maps.is_empty(), "no prepared maps to validate");
    let elapsed_msec = start.elapsed().as_millis();
    Ok(Report {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256,
        native_client_used: false,
        catalog_load_msec: 0,
        simulation_elapsed_msec: elapsed_msec,
        elapsed_msec,
        simulation_msec,
        passed: maps.iter().all(|r| r.passed),
        maps,
    })
}
fn simulate(state: &PreparedGameState, simulation_msec: u32, row: &mut MapReport) -> Result<()> {
    let state = state.for_bot_validation()?;
    let collision = state
        .collision
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("map collision missing"))?;
    let mut game = state
        .tdm_match()?
        .ok_or_else(|| anyhow::anyhow!("TDM world missing"))?;
    game.set_validation_seed(row.validation_seed);
    let local = state
        .local_player_snapshot(false)
        .ok_or_else(|| anyhow::anyhow!("local actor missing"))?;
    let weapon = state
        .match_bot_weapon()
        .ok_or_else(|| anyhow::anyhow!("Bot weapon missing"))?;
    let initial = game.snapshots();
    let expected_bots = usize::from(state.match_player_count() - 1);
    ensure!(
        initial.len() == expected_bots,
        "map Bot count differs from native preparation"
    );
    let mut previous = initial;
    let mut travelled = std::collections::BTreeMap::<u8, f32>::new();
    let mut moved = std::collections::BTreeSet::new();
    let mut dead = std::collections::BTreeSet::new();
    for now in (1000..=1000 + simulation_msec).step_by(50) {
        game.step_with_smoke(
            now,
            (local, 70.0, 0),
            weapon,
            &crate::game_smoke::Smoke::default(),
            collision,
            |s, f, fire, toggle| state.animate_enemy(s, f, fire, toggle),
        )?;
        let current_bots = game.snapshots();
        ensure!(current_bots.len() == expected_bots, "incomplete Bot roster");
        for (before, current) in previous.iter().zip(&current_bots) {
            ensure!(
                before.client_index == current.client_index,
                "Bot identity changed"
            );
            ensure!(
                current.origin.iter().all(|x| x.is_finite()),
                "non-finite Bot position"
            );
            ensure!(
                current.weapon != 0 && current.model != 0,
                "Bot model or weapon missing"
            );
            let distance = (0..2)
                .map(|i| (current.origin[i] - before.origin[i]).powi(2))
                .sum::<f32>()
                .sqrt();
            // A respawn warp is not evidence of navigation. Count only bounded
            // displacement between consecutive living snapshots.
            if before.alive && current.alive && distance <= 64.0 {
                let total = travelled.entry(current.client_index).or_default();
                *total += distance;
                if *total >= 128.0 {
                    moved.insert(current.client_index);
                }
            }
            if !current.alive {
                dead.insert(current.client_index);
            } else if dead.remove(&current.client_index) {
                row.respawns += 1;
            }
        }
        previous = current_bots;
        row.kills += game
            .take_kills()
            .iter()
            .filter(|k| !k.falling && k.attacker != k.victim.client_index)
            .count();
        if game.finished {
            break;
        }
    }
    row.moved_bots = moved.len();
    row.destroyed_glass = collision.destroyed_glass().len();
    ensure!(
        row.moved_bots >= 8.min(expected_bots),
        "only {} Bots travelled at least 128 units while alive",
        row.moved_bots
    );
    ensure!(
        row.kills > 0,
        "no combat kills during the simulated interval"
    );
    ensure!(
        row.respawns > 0,
        "no death/respawn cycle during the simulated interval"
    );
    let mut actors = game.snapshots();
    actors.push(local);
    let scene = state.top_players_scene(&game.scoreboard(0).rows, actors)?;
    ensure!(scene.poses.len() == 3, "three podium actors missing");
    row.top_players_scene = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_catalog_and_out_of_bounds_intervals_cannot_pass_validation() {
        let catalog = GameMapCatalog::default();
        for duration in [0, 14_999, 15_000, 120_000, 120_001, u32::MAX] {
            assert!(verify(&catalog, duration).is_err());
        }
    }
}
