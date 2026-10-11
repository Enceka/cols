//! Passive skills derived from this client's marketplace, perk and ability tables.
use anyhow::{Result, ensure};
use codol_protocol::game_abilities::Abilities;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    class_offsets: Vec<[usize; 8]>,
    skills: Vec<Skill>,
    abilities: Vec<[String; 7]>,
}
#[derive(Deserialize)]
struct Skill {
    item_id: u32,
    native_id: u32,
    slot: usize,
    name: String,
    modifiers: Vec<[String; 2]>,
    flag: String,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let catalog: Catalog = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/passive-skills-3.3.3.3.json"
    ))
    .expect("captured passive skill tables");
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(catalog.capture_sha256.len(), 64);
    assert_eq!(catalog.class_offsets.len(), 60);
    catalog
});

#[derive(Clone, Debug, Default)]
pub(crate) struct Perks {
    pub slots: [u32; 3],
    pub abilities: Abilities,
    /// Server-only values stay in the life/loadout rather than a client bitmask.
    values: BTreeMap<String, f32>,
    active: BTreeSet<String>,
}
impl Perks {
    pub fn value(&self, name: &str, default: f32) -> f32 {
        self.values.get(name).copied().unwrap_or(default)
    }
    pub fn has(&self, name: &str) -> bool {
        self.active.contains(name)
    }
    /// 0x508d59/0x508da2: PS+0x4c0 has 96 LSB-first perk-ID bits.
    /// This is separate from the 160 MSB-first ability bits at PS+0x4cc.
    pub fn native_masks(&self) -> [u32; 3] {
        let mut masks = [0; 3];
        for id in self.slots.into_iter().filter(|id| *id != 0) {
            assert!(id < 96, "captured native perk mask budget");
            masks[id as usize / 32] |= 1 << (id % 32);
        }
        masks
    }
    pub fn from_profile(snapshot: &[u8], class: u8, owned: &[u32]) -> Result<Self> {
        let offsets = CATALOG
            .class_offsets
            .get(usize::from(class))
            .ok_or_else(|| anyhow::anyhow!("invalid passive skill class"))?;
        let mut ids = [0; 3];
        for (slot, offset) in offsets[..6].iter().enumerate() {
            // Native 0x593040/0x593121..125: positive raw int -> raw-1;
            // 0 and 1 both resolve empty. Weapon/attachment IDs are different.
            let raw = snapshot
                .get(*offset..*offset + 4)
                .ok_or_else(|| anyhow::anyhow!("passive skill profile bounds"))?;
            let raw = i32::from_le_bytes(raw.try_into().unwrap());
            ensure!(raw >= 0, "negative passive skill reference");
            let id = (raw as u32).saturating_sub(1);
            if slot < 3 {
                ids[slot] = id;
            } else {
                ensure!(
                    id == 0,
                    "additional wildcard passive skills need their slot rules"
                );
            }
        }
        Self::selected(ids, owned)
    }
    pub(crate) fn selected(ids: [u32; 3], owned: &[u32]) -> Result<Self> {
        let mut result = Self::default();
        let mut deltas = BTreeMap::<String, f64>::new();
        for (slot, item) in ids.into_iter().enumerate().filter(|(_, id)| *id != 0) {
            let skill = CATALOG
                .skills
                .iter()
                .find(|s| s.item_id == item)
                .ok_or_else(|| anyhow::anyhow!("unknown passive skill item {item}"))?;
            ensure!(owned.contains(&item), "unowned passive skill {item}");
            ensure!(
                skill.slot == slot,
                "passive skill {} has wrong color",
                skill.name
            );
            result.slots[slot] = skill.native_id;
            for [name, delta] in &skill.modifiers {
                let delta = if delta.is_empty() {
                    0.0
                } else {
                    delta.parse::<f64>()?
                };
                *deltas.entry(name.clone()).or_default() += delta;
            }
            if !skill.flag.is_empty() {
                deltas.entry(skill.flag.clone()).or_default();
            }
        }
        for (name, delta) in deltas {
            let row = CATALOG
                .abilities
                .iter()
                .find(|r| r[1] == name)
                .ok_or_else(|| anyhow::anyhow!("unknown passive ability {name}"))?;
            let id = row[0].parse::<usize>()?;
            ensure!(id < 160, "native ability mask limit");
            let parse = |cell: &str| -> Result<Option<f64>> {
                if cell.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(cell.parse()?))
                }
            };
            let mut value = parse(&row[3])?.unwrap_or(0.0) + delta;
            if let Some(min) = parse(&row[4])? {
                value = value.max(min);
            }
            if let Some(max) = parse(&row[5])? {
                value = value.min(max);
            }
            ensure!(
                value.is_finite() && (value as f32).is_finite(),
                "invalid passive value"
            );
            result.active.insert(name.clone());
            result.values.insert(name, value as f32);
            if row[6] == "1" {
                result.abilities.masks[id / 32] |= 0x80000000 >> (id % 32);
            } else {
                ensure!(row[6] == "0", "invalid ability publication flag");
            }
            if !row[2].is_empty() {
                let slot = row[2].parse::<usize>()?;
                ensure!(slot < 30, "native ability value limit");
                result.abilities.values[slot] = value as f32;
            }
        }
        result.abilities.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_backpack_decodes_item_plus_one_and_server_only_resistance() {
        let mut profile = vec![0; 32768];
        for (slot, raw) in [60044u32, 60041, 60138].into_iter().enumerate() {
            let offset = CATALOG.class_offsets[59][slot];
            profile[offset..offset + 4].copy_from_slice(&raw.to_le_bytes());
        }
        let perks = Perks::from_profile(&profile, 59, &[60043, 60040, 60137]).unwrap();
        assert_eq!(perks.slots, [9, 2, 58]);
        assert_eq!(perks.native_masks(), [0x204, 0x04000000, 0]);
        assert_eq!(
            perks.abilities.masks[0],
            (0x80000000 >> 20) | (0x80000000 >> 2)
        );
        // modNoHelicopter131 and modNoAirplane132 are server-only, beyond C1's128 limit.
        assert!(perks.has("modNoHelicopter") && perks.has("modNoAirplane"));
        assert_eq!(perks.abilities.masks[4], 0);
        assert!((perks.value("modFlashProtectionScale", 1.0) - 0.1).abs() < 1e-6);
        assert!(Perks::from_profile(&profile, 59, &[60043, 60040]).is_err());
    }
    #[test]
    fn captured_colors_and_shared_modifiers_are_validated_and_aggregated() {
        let p = Perks::selected([60075, 60074, 60215], &[60075, 60074, 60215]).unwrap();
        assert_eq!(p.value("modFootstepVolumeScaleEnemy", 1.0), 3.0);
        assert_eq!(p.value("modFootstepVolumeScaleAlly", 1.0), 0.0);
        assert_eq!(p.abilities.values[11], 1.5);
        assert_eq!(p.abilities.values[14], 1.6);
        assert!(Perks::selected([60040, 0, 0], &[60040]).is_err());
        for skill in &CATALOG.skills {
            let mut ids = [0; 3];
            ids[skill.slot] = skill.item_id;
            let perks = Perks::selected(ids, &[skill.item_id]).unwrap();
            let mask = perks.native_masks();
            assert_eq!(mask.iter().map(|word| word.count_ones()).sum::<u32>(), 1);
            assert_eq!(
                mask[skill.native_id as usize / 32],
                1 << (skill.native_id % 32)
            );
        }
    }
}
