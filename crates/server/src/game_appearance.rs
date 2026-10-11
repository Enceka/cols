//! Third-person composition from the current client's retained character banks.
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const KINDS: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 11];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    schema_version: u8,
    client_sha256: String,
    sources_sha256: Vec<String>,
    current_character_offset: usize,
    slot_offsets: [[usize; 9]; 12],
    items: Vec<Item>,
    packages: Vec<Package>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: u16,
    kind: u8,
    variants: Vec<Variant>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    mask: u32,
    model: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    id: u16,
    gender: u8,
    skin: u8,
    defaults: [u16; 12],
    allowed: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Appearance {
    pub package: u16,
    pub models: Vec<String>,
    pub indices: Vec<u16>,
    pub registrations: Vec<(u16, String)>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            path.metadata()?.len() <= 4 * 1024 * 1024,
            "appearance catalog exceeds budget"
        );
        let catalog: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        catalog.validate()?;
        Ok(catalog)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.client_sha256 == codol_protocol::CLIENT_SHA256,
            "appearance build identity mismatch"
        );
        ensure!(
            self.sources_sha256.len() == 2
                && self
                    .sources_sha256
                    .iter()
                    .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            "appearance sources missing"
        );
        ensure!(
            (12..32768).contains(&self.current_character_offset),
            "appearance selector outside profile"
        );
        let mut offsets = BTreeSet::from([self.current_character_offset]);
        for &offset in self.slot_offsets.iter().flatten() {
            ensure!(
                (12..=32764).contains(&offset) && (offset..offset + 4).all(|b| offsets.insert(b)),
                "appearance slots overlap/outside profile"
            );
        }
        ensure!(
            !self.items.is_empty()
                && self.items.len() <= 4096
                && !self.packages.is_empty()
                && self.packages.len() <= 256,
            "appearance catalog counts"
        );
        let mut ids = BTreeSet::new();
        for item in &self.items {
            ensure!(
                item.id != 0
                    && ids.insert(item.id)
                    && KINDS.contains(&item.kind)
                    && item.variants.len() <= 32,
                "invalid appearance item"
            );
            for v in &item.variants {
                ensure!(
                    v.mask != 0
                        && !v.model.is_empty()
                        && v.model.len() < 64
                        && v.model
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                    "invalid appearance model"
                );
            }
        }
        for p in &self.packages {
            ensure!(
                p.id != 0
                    && ids.insert(p.id)
                    && p.gender <= 1
                    && p.skin <= 3
                    && p.allowed.len() <= 1000,
                "invalid appearance package"
            );
            ensure!(
                p.defaults[0] == 0 && p.defaults[10] == 0,
                "package contains non-body defaults"
            );
            ensure!(
                p.defaults[1..=5].iter().all(|&i| i != 0),
                "package lacks mandatory body parts"
            );
            for (kind, &id) in p.defaults.iter().enumerate().filter(|(_, id)| **id != 0) {
                ensure!(
                    self.items
                        .iter()
                        .any(|i| i.id == id && usize::from(i.kind) == kind),
                    "package default has wrong item kind"
                );
            }
        }
        Ok(())
    }
    pub fn compose(&self, profile: &[u8], owned: &[u32], team: u8) -> Result<Appearance> {
        ensure!(
            profile.len() == 32768 && matches!(team, 1 | 2),
            "invalid appearance profile/team"
        );
        let character = usize::from(profile[self.current_character_offset]);
        ensure!(character < 12, "character selector outside profile");
        let slots: Vec<u32> = self.slot_offsets[character]
            .iter()
            .map(|&o| u32::from_le_bytes(profile[o..o + 4].try_into().unwrap()))
            .collect();
        let package = self
            .packages
            .iter()
            .find(|p| u32::from(p.id) == slots[0])
            .ok_or_else(|| anyhow::anyhow!("selected character package is unavailable"))?;
        ensure!(
            owned.contains(&u32::from(package.id)),
            "selected character is not owned"
        );
        let mut selected = package.defaults;
        let mut kinds = BTreeSet::new();
        for id in slots
            .into_iter()
            .skip(1)
            .filter(|&i| i != 0 && i != u32::MAX)
        {
            let item = self
                .items
                .iter()
                .find(|i| u32::from(i.id) == id)
                .ok_or_else(|| anyhow::anyhow!("selected wearable is unavailable"))?;
            ensure!(
                owned.contains(&id) && kinds.insert(item.kind),
                "unowned or duplicate wearable kind"
            );
            ensure!(
                package.allowed.contains(&item.id) || package.defaults.contains(&item.id),
                "wearable does not belong to selected character"
            );
            selected[usize::from(item.kind)] = item.id;
        }
        // 0x40c980/0x40c800: first variant whose mask includes all three
        // requested bits. Shared gender/faction/tone entries contain more bits.
        let skin = match package.skin {
            0 => 5,
            1 => 8,
            2 => 6,
            _ => 7,
        };
        let mask = (0x8000_0000u32 >> package.gender)
            | (0x8000_0000u32 >> if team == 1 { 3 } else { 2 })
            | (0x8000_0000u32 >> skin);
        let mut models = Vec::new();
        for kind in KINDS {
            let id = selected[usize::from(kind)];
            if id == 0 {
                continue;
            }
            let item = self.items.iter().find(|i| i.id == id).unwrap();
            let Some(variant) = item.variants.iter().find(|v| v.mask & mask == mask) else {
                // 0x40c980 keeps unmatched groups at zero; optional parts can
                // intentionally have a variant for only one faction.
                ensure!(
                    kind > 5,
                    "wearable {id} lacks a mandatory body model for selected character/team"
                );
                continue;
            };
            let model = &variant.model;
            models.push(model.to_ascii_lowercase());
        }
        ensure!(
            (5..=10).contains(&models.len()) && models[0].starts_with("fpa_"),
            "incomplete character body"
        );
        Ok(Appearance {
            package: package.id,
            models,
            indices: Vec::new(),
            registrations: Vec::new(),
        })
    }
}
impl Appearance {
    pub fn register(&mut self, known: &[(u16, String)], first: u16) -> Result<()> {
        let mut by_name: BTreeMap<_, _> = known
            .iter()
            .map(|(i, n)| (n.to_ascii_lowercase(), *i))
            .collect();
        let mut next = known
            .iter()
            .map(|(i, _)| i + 1)
            .max()
            .unwrap_or(first)
            .max(first);
        self.indices.clear();
        self.registrations.clear();
        for name in &self.models {
            let index = if let Some(&i) = by_name.get(name) {
                i
            } else {
                ensure!(next < 1024, "character model registry exhausted");
                let id = next;
                next += 1;
                by_name.insert(name.clone(), id);
                self.registrations.push((id, name.clone()));
                id
            };
            self.indices.push(index);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Catalog {
        Catalog {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.into(),
            sources_sha256: vec!["0".repeat(64); 2],
            current_character_offset: 12,
            slot_offsets: std::array::from_fn(|c| std::array::from_fn(|s| 16 + (c * 9 + s) * 4)),
            packages: vec![Package {
                id: 792,
                gender: 0,
                skin: 0,
                defaults: [0, 101, 102, 103, 104, 105, 0, 0, 0, 0, 0, 0],
                allowed: vec![106],
            }],
            items: (101..=106)
                .map(|id| Item {
                    id,
                    kind: if id == 106 { 3 } else { (id - 100) as u8 },
                    variants: vec![
                        Variant {
                            mask: 0xa4000000,
                            model: format!("{}_tf_{id}", if id == 101 { "fpa" } else { "part" }),
                        },
                        Variant {
                            mask: 0x94000000,
                            model: format!("{}_sc_{id}", if id == 101 { "fpa" } else { "part" }),
                        },
                    ],
                })
                .collect(),
        }
    }
    fn profile(c: &Catalog, character: usize, package: u32) -> Vec<u8> {
        let mut p = vec![0; 32768];
        p[c.current_character_offset] = character as u8;
        p[c.slot_offsets[character][0]..][..4].copy_from_slice(&package.to_le_bytes());
        p
    }
    #[test]
    fn selected_slot_package_defaults_and_owned_kind_overrides_compose_per_team() {
        let c = catalog();
        c.validate().unwrap();
        let mut p = profile(&c, 7, 792);
        p[c.slot_offsets[0][0]..][..4].copy_from_slice(&999u32.to_le_bytes());
        let default = c.compose(&p, &[792], 2).unwrap();
        assert_eq!(default.models[0], "fpa_tf_101");
        let offset = c.slot_offsets[7][6];
        p[offset..offset + 4].copy_from_slice(&106u32.to_le_bytes());
        assert!(c.compose(&p, &[792], 2).is_err());
        let mut equipped = c.compose(&p, &[792, 106], 1).unwrap();
        assert_eq!(equipped.models[2], "part_sc_106");
        equipped
            .register(&[(150, "part_sc_106".into())], 200)
            .unwrap();
        assert_eq!(equipped.indices, vec![200, 201, 150, 202, 203]);
        assert_eq!(equipped.registrations.len(), 4);
        p[c.slot_offsets[7][8]..][..4].copy_from_slice(&106u32.to_le_bytes());
        assert!(c.compose(&p, &[792, 106], 2).is_err());
    }
    #[test]
    fn unsupported_variants_and_catalog_layout_fail_without_substituting_characters() {
        let mut c = catalog();
        let p = profile(&c, 0, 792);
        c.packages[0].gender = 1;
        assert!(c.compose(&p, &[792], 2).is_err());
        c.items[0].variants[0].mask = 0xe4000000;
        for i in &mut c.items {
            i.variants[0].mask = 0xe4000000;
        }
        assert!(c.compose(&p, &[792], 2).is_ok());
        c.slot_offsets[1][0] = c.slot_offsets[0][0];
        assert!(c.validate().is_err());
        let mut c = catalog();
        c.items[0].variants[0].model = "bad;quit".into();
        assert!(c.validate().is_err());
    }
    #[test]
    #[ignore = "requires local appearance catalog and captured public profile"]
    fn captured_outfits_compose_without_touching_the_saved_profile() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let c = Catalog::load(&root.join("player-appearance-catalog.json")).unwrap();
        let mut summary = Vec::new();
        for package in &c.packages {
            let p = profile(&c, 0, u32::from(package.id));
            for team in [1, 2] {
                let a = c.compose(&p, &[u32::from(package.id)], team);
                summary.push(serde_json::json!({"package":package.id,"team":team,"gender":package.gender,
                    "models":a.as_ref().ok().map(|a|&a.models),"error":a.err().map(|e|e.to_string())}));
            }
        }
        std::fs::write(
            root.join("appearance-defaults-proof.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
        assert!(
            summary.iter().all(|r| r["error"].is_null()),
            "default appearance composition failed"
        );
        for name in [
            "tdm-afghan-hud-state",
            "tdm-killhouse-hud-state",
            "tdm-crash-tropical-state",
        ] {
            let state = crate::game_state::PreparedGameState::load(
                &root.join(format!("{name}-appearance.json")),
            )
            .unwrap();
            let path = std::env::var_os("CODOL_APPEARANCE_PROFILE")
                .expect("local profile directory required");
            let p = Path::new(&path);
            let public = std::fs::read(p.join("public.bin")).unwrap();
            let inventory: serde_json::Value =
                serde_json::from_slice(&std::fs::read(p.join("inventory.json")).unwrap()).unwrap();
            let owned = serde_json::from_value::<Vec<u32>>(inventory["items"].clone()).unwrap();
            let resolved = state.for_profile(&public, &owned).unwrap();
            let actor = resolved.local_player_snapshot(true).unwrap();
            let gamestate = resolved.body(1).unwrap();
            let wire =
                codol_protocol::game_wire::compact_server_packet(3, 0, 0, b"", &gamestate).unwrap();
            assert!(actor.model >= 200);
            assert_eq!(
                actor.attachment_models.iter().filter(|&&i| i > 0).count(),
                5
            );
            let game = resolved.tdm_match().unwrap().unwrap();
            let players = game.snapshots();
            let body = resolved
                .snapshot(
                    1000,
                    1000,
                    true,
                    [0; 3],
                    resolved.player().map(|p| p.equipment()),
                    crate::game_state::SnapshotWorld {
                        motion: resolved.motion().as_ref(),
                        local_player: Some(actor),
                        players: &players,
                        vitals: Some(Default::default()),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(body.len() <= codol_protocol::game_wire::FRAGMENT_SIZE - 5);
            let mut later = state.clone();
            later.preserve_match_appearance(&resolved);
            assert_eq!(
                later.local_player_snapshot(true).unwrap().attachment_models,
                actor.attachment_models
            );
            eprintln!(
                "{name}: model={} attachments={:?} full_snapshot={} gamestate={} wire={}",
                actor.model,
                actor.attachment_models,
                body.len(),
                gamestate.len(),
                wire.len()
            );
        }
    }
}
