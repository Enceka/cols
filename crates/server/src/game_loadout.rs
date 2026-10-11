//! Resolve the saved primary backpack against target item/attachment records.
//! Training currently chooses the configured backpack (zero-based), not a UI
//! selection guessed from currentCharacterIndex. Runtime variants keep timing
//! changes explicit until the complete attachment-stat evaluator is ported.
use super::{BootstrapAmmo, BootstrapWeapon, PreparedGameState};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Part {
    item_id: u32,
    kind: u8,
    name: String,
    wire_index: u16,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Variant {
    attachments: Vec<u16>,
    ammo: BootstrapAmmo,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Loadout {
    class_index: u8,
    primary_offset: usize,
    attachment_offsets: [usize; 6],
    #[serde(default)]
    secondary_offset: Option<usize>,
    #[serde(default)]
    secondary_attachment_offsets: Option<[usize; 6]>,
    projection_offsets: Vec<usize>,
    schema_source_sha256: String,
    parts_source_sha256: String,
    #[serde(default)]
    weapon_item_id: u32,
    #[serde(default)]
    defaults: Vec<Part>,
    #[serde(default)]
    compatible: Vec<Part>,
    #[serde(default)]
    variants: Vec<Variant>,
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
impl Loadout {
    pub(super) fn profile_snapshot(&self, public: &[u8]) -> Result<Vec<u8>> {
        Ok(codol_protocol::profile::public_snapshot(
            public,
            &self.projection_offsets,
        )?)
    }
    pub(super) fn card(&self, public: &[u8]) -> Result<codol_protocol::game_player_card::Card> {
        Ok(codol_protocol::game_player_card::Card::from_profile(
            public,
            &self.projection_offsets,
        )?)
    }
    pub(super) fn selected_for(&self, public: &[u8], slot: u8) -> Result<Self> {
        let snapshot = codol_protocol::profile::public_snapshot(public, &self.projection_offsets)?;
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/backpack-class-offsets-3.3.3.3.json"
        ))?;
        let character = snapshot[layout["current_character_offset"].as_u64().unwrap() as usize];
        let index = crate::game_class_selection::global_class_index(character, slot)?;
        let offsets: Vec<usize> =
            serde_json::from_value(layout["class_offsets"][usize::from(index)].clone())?;
        let mut selected = self.clone();
        selected.class_index = index;
        selected.primary_offset = offsets[0];
        selected.attachment_offsets = offsets[1..7].try_into().unwrap();
        selected.secondary_offset = Some(offsets[7]);
        selected.secondary_attachment_offsets = Some(offsets[8..14].try_into().unwrap());
        Ok(selected)
    }
    pub(super) fn class_index(&self) -> u8 {
        self.class_index
    }
    pub(super) fn validate(&self, state: &PreparedGameState) -> Result<()> {
        let weapon = state
            .bootstrap_weapon
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("loadout requires bootstrap weapon"))?;
        ensure!(
            weapon.base_index.is_some() && weapon.camo == Some(0),
            "loadout requires composed uncamoed M4"
        );
        ensure!(
            self.class_index < 60
                && (state.dynamic_loadout() || self.weapon_item_id == 10001)
                && self.projection_offsets.len() == 125
                && self
                    .projection_offsets
                    .iter()
                    .all(|&o| (12..=14426).contains(&o))
                && hash_valid(&self.schema_source_sha256)
                && hash_valid(&self.parts_source_sha256),
            "loadout schema/evidence mismatch"
        );
        let offsets = std::iter::once(self.primary_offset)
            .chain(self.attachment_offsets)
            .chain(self.secondary_offset)
            .chain(self.secondary_attachment_offsets.into_iter().flatten());
        let mut bytes = BTreeSet::new();
        for offset in offsets {
            ensure!(
                (12..=14426).contains(&offset) && (offset..offset + 4).all(|o| bytes.insert(o)),
                "loadout fields overlap/out of bounds"
            );
        }
        ensure!(
            self.secondary_offset.is_some() == self.secondary_attachment_offsets.is_some(),
            "incomplete secondary profile layout"
        );
        if state.dynamic_loadout() {
            return Ok(());
        }
        ensure!(
            !self.defaults.is_empty()
                && self.defaults.len() <= 16
                && self.compatible.len() <= 128
                && !self.variants.is_empty()
                && self.variants.len() <= 64,
            "loadout catalogue size"
        );
        let mut ids = BTreeSet::new();
        for part in self.defaults.iter().chain(&self.compatible) {
            ensure!(
                part.item_id > 0
                    && part.item_id <= 65535
                    && part.item_id != 20008
                    && ids.insert(part.item_id)
                    && part.kind < 16
                    && (1..0x3fff).contains(&part.wire_index)
                    && !part.name.is_empty()
                    && part.name.len() <= 64
                    && part
                        .name
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'),
                "invalid loadout item"
            );
        }
        ensure!(
            self.defaults
                .iter()
                .map(|p| p.kind)
                .collect::<BTreeSet<_>>()
                .len()
                == self.defaults.len(),
            "duplicate default slot"
        );
        let known = self
            .defaults
            .iter()
            .chain(&self.compatible)
            .map(|p| p.wire_index)
            .collect::<BTreeSet<_>>();
        let mut variants = BTreeSet::new();
        for variant in &self.variants {
            ensure!(
                variant.attachments.iter().all(|id| known.contains(id))
                    && variants.insert(variant.attachments.clone()),
                "unknown or duplicate loadout variant"
            );
            let mut candidate = state.clone();
            candidate.player_loadout = None;
            let w = candidate.bootstrap_weapon.as_mut().unwrap();
            w.attachments = variant.attachments.clone();
            w.composed_name = Some(codol_protocol::game_state::composed_weapon(
                w.base_index.unwrap(),
                0,
                &w.attachments,
            )?);
            w.attachment_source_sha256 = Some(self.parts_source_sha256.clone());
            w.ammo = Some(variant.ammo.clone());
            ensure!(
                variant.ammo.clip_key == w.index
                    && variant.ammo.reserve_key == w.base_index.unwrap()
                    && variant.ammo.fire_interval_msec.is_some()
                    && variant.ammo.reload.is_some()
                    && variant.ammo.ads.is_some(),
                "loadout variant requires measured ammo/reload/ADS"
            );
            candidate.validate()?;
        }
        Ok(())
    }
    pub(super) fn selections(&self, public: &[u8]) -> Result<[(u32, Vec<u32>); 2]> {
        let plain = codol_protocol::profile::public_snapshot(public, &self.projection_offsets)?;
        let read = |o: usize| u32::from_le_bytes(plain[o..o + 4].try_into().unwrap());
        Ok([
            (
                read(self.primary_offset),
                self.attachment_offsets.map(read).to_vec(),
            ),
            (
                self.secondary_offset.map_or(0, read),
                self.secondary_attachment_offsets
                    .map_or(Vec::new(), |offsets| offsets.map(read).to_vec()),
            ),
        ])
    }
    pub(super) fn resolve(
        &self,
        state: &PreparedGameState,
        public: &[u8],
        owned: &[u32],
    ) -> Result<BootstrapWeapon> {
        let snapshot = codol_protocol::profile::public_snapshot(public, &self.projection_offsets)?;
        let item =
            |offset: usize| u32::from_le_bytes(snapshot[offset..offset + 4].try_into().unwrap());
        ensure!(
            item(self.primary_offset) == self.weapon_item_id,
            "selected backpack primary is not the supported M4"
        );
        ensure!(
            owned.contains(&self.weapon_item_id),
            "player does not own backpack weapon"
        );
        let mut parts = self
            .defaults
            .iter()
            .map(|p| (p.kind, p))
            .collect::<BTreeMap<_, _>>();
        let mut selected_kinds = BTreeSet::new();
        let mut selected = Vec::new();
        for offset in self.attachment_offsets {
            let id = item(offset);
            if id == 0 || id == 20008 {
                continue;
            }
            let part = self
                .compatible
                .iter()
                .find(|p| p.item_id == id)
                .ok_or_else(|| {
                    anyhow::anyhow!("backpack attachment {id} is not compatible with M4")
                })?;
            ensure!(
                owned.contains(&id),
                "player does not own backpack attachment {id}"
            );
            ensure!(
                selected_kinds.insert(part.kind),
                "backpack has conflicting attachment slots"
            );
            parts.insert(part.kind, part);
            selected.push(id);
        }
        let attachments = parts.values().map(|p| p.wire_index).collect::<Vec<_>>();
        let variant = self
            .variants
            .iter()
            .find(|v| v.attachments == attachments)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "backpack combination has no measured runtime parameters: {selected:?}"
                )
            })?;
        let mut weapon = state.bootstrap_weapon.clone().unwrap();
        weapon.composed_name = Some(codol_protocol::game_state::composed_weapon(
            weapon.base_index.unwrap(),
            0,
            &attachments,
        )?);
        weapon.attachments = attachments;
        weapon.attachment_source_sha256 = Some(self.parts_source_sha256.clone());
        weapon.ammo = Some(variant.ammo.clone());
        tracing::info!(backpack = self.class_index + 1, selected_items = ?selected, composed_name = ?weapon.composed_name, "resolved saved player loadout");
        Ok(weapon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codol_protocol::task::Writer;

    fn profile(primary: u32, selected: [u32; 6]) -> Vec<u8> {
        let mut plain = vec![0u8; 32768];
        plain[1000..1004].copy_from_slice(&primary.to_le_bytes());
        for (i, id) in selected.iter().enumerate() {
            plain[1004 + i * 4..1008 + i * 4].copy_from_slice(&id.to_le_bytes());
        }
        let crc = crc32fast::hash(&plain[4..]);
        plain[..4].copy_from_slice(&crc.to_le_bytes());
        let mut w = Writer::default();
        w.u32(0);
        w.u32(1);
        w.u32(crc);
        w.blob(&plain).unwrap();
        for _ in 0..125 {
            w.i32(0);
        }
        w.0.push(0);
        w.0
    }
    fn state() -> PreparedGameState {
        let mut state = super::super::synthetic();
        state.spawn_origin = Some([0.0; 3]);
        state.map_entities_sha256 = Some("a".repeat(64));
        let ammo = BootstrapAmmo {
            clip_key: 352,
            reserve_key: 163,
            clip: 30,
            reserve: 90,
            source_sha256: "a".repeat(64),
            fire_interval_msec: Some(79),
            reload: Some(crate::game_player::ReloadTiming {
                normal_msec: 2029,
                empty_msec: 2799,
                add_msec: 1399,
                empty_add_msec: 1399,
                segmented: None,
            }),
            ads: Some(crate::game_player::AdsTiming {
                in_rate: 0.004,
                out_rate: 0.004,
                reload_allowed: false,
                reload_interruptible: false,
                reload_end_msec: 349,
            }),
        };
        state.bootstrap_weapon = Some(BootstrapWeapon {
            name: "m4_mp".into(),
            index: 352,
            ammo: Some(ammo.clone()),
            base_index: Some(163),
            composed_name: Some("** 163 0 0 10 11".into()),
            viewmodel_index: Some(54),
            attachments: vec![10, 11],
            attachment_source_sha256: Some("a".repeat(64)),
            camo: Some(0),
            switch: None,
            move_scale: None,
            fire_type: None,
            alternate_base: None,
            alternate: None,
            alternate_timing: None,
            rechamber: None,
            spread: None,
            ballistics: None,
        });
        let part = |item_id, kind, wire_index| Part {
            item_id,
            kind,
            wire_index,
            name: format!("part_{item_id}"),
        };
        let mut modified_ammo = ammo.clone();
        modified_ammo.reload.as_mut().unwrap().normal_msec = 2200;
        state.player_loadout = Some(Loadout {
            class_index: 0,
            primary_offset: 1000,
            attachment_offsets: [1004, 1008, 1012, 1016, 1020, 1024],
            secondary_offset: None,
            secondary_attachment_offsets: None,
            projection_offsets: (0..125).map(|i| 12 + i * 4).collect(),
            schema_source_sha256: "a".repeat(64),
            parts_source_sha256: "b".repeat(64),
            weapon_item_id: 10001,
            defaults: vec![part(20032, 4, 10), part(20031, 9, 11)],
            compatible: vec![
                part(20659, 4, 20),
                part(21237, 9, 21),
                part(20782, 5, 22),
                part(20660, 4, 23),
            ],
            variants: vec![
                Variant {
                    attachments: vec![10, 11],
                    ammo,
                },
                Variant {
                    attachments: vec![20, 21],
                    ammo: modified_ammo,
                },
            ],
        });
        state
    }
    #[test]
    fn reconnect_replays_history_and_keeps_secondary_identity_after_primary_changes() {
        let mut initial = state();
        let mut secondary = initial.bootstrap_weapon.clone().unwrap();
        secondary.name = "m9_mp".into();
        secondary.base_index = Some(180);
        secondary.composed_name = Some("** 180 0 0 30 31".into());
        secondary.attachments = vec![30, 31];
        secondary.ammo.as_mut().unwrap().reserve_key = 180;
        initial.secondary_weapon = Some(secondary);
        initial.player_loadout = None;
        let mut registry = crate::game_weapon_registry::WeaponRegistry::new(352);
        initial.register_history(&mut registry).unwrap();
        assert_eq!(initial.secondary_weapon.as_ref().unwrap().index, 353);
        let mut changed = initial.clone();
        let primary = changed.bootstrap_weapon.as_mut().unwrap();
        primary.attachments = vec![20, 21];
        primary.composed_name = Some("** 163 0 0 20 21".into());
        changed.register_history(&mut registry).unwrap();
        assert_eq!(changed.bootstrap_weapon.as_ref().unwrap().index, 354);
        assert_eq!(changed.secondary_weapon.as_ref().unwrap().index, 353);
        // A fresh connection gets every registered name in index order, even
        // when only the initial primary and secondary are currently equipped.
        let mut reentered = initial;
        reentered.register_history(&mut registry).unwrap();
        assert_eq!(reentered.bootstrap_weapon.as_ref().unwrap().index, 352);
        assert_eq!(reentered.secondary_weapon.as_ref().unwrap().index, 353);
        assert_eq!(reentered.next_weapon_index(), 355);
        assert_eq!(
            reentered.weapon_configstrings(),
            vec![
                (5795, "** 163 0 0 10 11".into()),
                (5796, "** 180 0 0 30 31".into()),
                (5797, "** 163 0 0 20 21".into()),
            ]
        );
        let body = reentered.body(1).unwrap();
        codol_protocol::game_wire::server_packet(3, 0, 0, b"", &body).unwrap();
    }
    #[test]
    fn automatically_composed_alternate_reserves_its_slot_before_the_secondary() {
        let mut state = state();
        state.player_loadout = None;
        let parent = state.bootstrap_weapon.as_mut().unwrap();
        parent.alternate_base = Some(31);
        let mut secondary = parent.clone();
        secondary.alternate_base = None;
        secondary.base_index = Some(180);
        secondary.composed_name = Some("** 180 0 0 10 11".into());
        state.secondary_weapon = Some(secondary);
        let mut registry = crate::game_weapon_registry::WeaponRegistry::new(352);
        state.register_history(&mut registry).unwrap();
        assert_eq!(state.bootstrap_weapon.as_ref().unwrap().index, 352);
        assert_eq!(registry.names["** 31 0 0 10 11"], 353);
        assert_eq!(state.secondary_weapon.as_ref().unwrap().index, 354);
        assert_eq!(state.next_weapon_index(), 355);
        state.register_history(&mut registry).unwrap();
        assert_eq!(registry.names.len(), 3);
        assert_eq!(state.secondary_weapon.as_ref().unwrap().index, 354);
    }
    #[test]
    fn resource_updates_reuse_known_names_including_unchanged_secondary_and_returned_primary() {
        let initial = state();
        let mut known = initial
            .weapon_configstrings()
            .into_iter()
            .map(|(i, n)| (n, i - 5443))
            .collect();
        let mut next = 353;
        let mut changed = initial
            .for_profile(
                &profile(10001, [20659, 21237, 0, 0, 0, 0]),
                &[10001, 20659, 21237],
            )
            .unwrap();
        changed.secondary_weapon = initial.bootstrap_weapon.clone();
        let added = changed.register_weapons(&mut known, &mut next).unwrap();
        assert_eq!(added, [(5796, "** 163 0 0 20 21".into())]);
        assert_eq!(changed.bootstrap_weapon.as_ref().unwrap().index, 353);
        assert_eq!(changed.secondary_weapon.as_ref().unwrap().index, 352);
        assert_eq!(
            changed
                .bootstrap_weapon
                .as_ref()
                .unwrap()
                .ammo
                .as_ref()
                .unwrap()
                .clip_key,
            353
        );
        let mut returned = initial.clone();
        assert!(
            returned
                .register_weapons(&mut known, &mut next)
                .unwrap()
                .is_empty()
        );
        assert_eq!(returned.bootstrap_weapon.unwrap().index, 352);
        assert_eq!(next, 354);
    }
    #[test]
    fn saved_items_replace_same_slots_and_supply_their_measured_physics_per_player() {
        let state = state();
        state.validate().unwrap();
        let a = state
            .for_profile(
                &profile(10001, [20659, 21237, 0, 0, 0, 0]),
                &[10001, 20659, 21237],
            )
            .unwrap();
        let b = state
            .for_profile(&profile(10001, [0; 6]), &[10001])
            .unwrap();
        let a = a.bootstrap_weapon.unwrap();
        assert_eq!(a.composed_name.as_deref(), Some("** 163 0 0 20 21"));
        assert_eq!(a.ammo.unwrap().reload.unwrap().normal_msec, 2200);
        assert_eq!(b.bootstrap_weapon.unwrap().attachments, vec![10, 11]);
        assert_eq!(state.bootstrap_weapon.unwrap().attachments, vec![10, 11]);
    }
    #[test]
    fn no_default_fallback_for_unowned_unknown_conflicting_or_unmeasured_items() {
        let state = state();
        let cases = [
            ([20659, 21237, 0, 0, 0, 0], vec![10001]),
            ([99999, 0, 0, 0, 0, 0], vec![10001, 99999]),
            ([20659, 20660, 0, 0, 0, 0], vec![10001, 20659, 20660]),
            ([20782, 0, 0, 0, 0, 0], vec![10001, 20782]),
        ];
        for (parts, owned) in cases {
            assert!(state.for_profile(&profile(10001, parts), &owned).is_err());
        }
        assert!(
            state
                .for_profile(&profile(10010, [0; 6]), &[10001, 10010])
                .is_err()
        );
        assert!(state.for_profile(&profile(10001, [0; 6]), &[]).is_err());
    }
    #[test]
    fn profile_crc_and_projections_and_catalogue_layout_are_checked() {
        let mut state = state();
        let mut bad = profile(10001, [0; 6]);
        bad[21 + 1000] ^= 1;
        assert!(state.for_profile(&bad, &[10001]).is_err());
        let mut bad = profile(10001, [0; 6]);
        let projection = bad.len() - 625;
        bad[projection] = 1;
        assert!(state.for_profile(&bad, &[10001]).is_err());
        state.player_loadout.as_mut().unwrap().attachment_offsets[0] = 1000;
        assert!(state.validate().is_err());
    }
}
