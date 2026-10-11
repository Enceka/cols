//! Resolve owned backpack items using the target's captured weapon registry.
//! The compact item lookup has aliases; the independent marketplace table
//! establishes which numeric item IDs actually name each weapon/attachment.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Arsenal {
    pub(crate) source_sha256: String,
    registered_count: u16,
    weapons: BTreeMap<u16, Weapon>,
    item_to_base: BTreeMap<u32, u16>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedWeapon {
    pub(crate) name: String,
    pub(crate) base_index: u16,
    pub(crate) index: u16,
    pub(crate) attachments: Vec<u16>,
    pub(crate) composed_name: String,
    pub(crate) definition: Vec<u8>,
    pub(crate) melee_charge_animation: bool,
}

#[derive(Clone, Debug)]
struct Weapon {
    name: String,
    definition: Vec<u8>,
    melee_charge_animation: bool,
    defaults: BTreeMap<u8, Part>,
    items: BTreeMap<u32, Part>,
    attributes: BTreeMap<u16, Vec<[u8; 12]>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Part {
    item_id: u32,
    kind: u8,
    name: String,
    wire_index: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    schema_version: u32,
    read_only: bool,
    client_sha256: String,
    connection_state: u32,
    ncs_crc: u32,
    registered_count: u16,
    marketplace: Table,
    weapons: Vec<CapturedWeapon>,
}

#[derive(Deserialize)]
struct Table {
    #[serde(default)]
    found: Option<bool>,
    columns: usize,
    rows: usize,
    cells_base64: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedWeapon {
    name: String,
    base_index: u16,
    #[serde(default)]
    alternate_only: bool,
    #[serde(default)]
    item_ids: Vec<u32>,
    definition_base64: String,
    #[serde(default)]
    melee_charge_animation: Option<String>,
    defaults: Option<Defaults>,
    compatible: Vec<Compatible>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Defaults {
    weapon: String,
    parts: Vec<Part>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compatible {
    name: String,
    wire_index: u16,
    items: Vec<Part>,
    attributes_base64: String,
}

struct MarketItem {
    name: String,
    category: String,
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/'))
}

fn decode_cell(value: &str) -> Result<String> {
    let bytes = STANDARD
        .decode(value)
        .context("invalid marketplace cell base64")?;
    ensure!(
        bytes.len() <= 4096 && bytes.is_ascii(),
        "invalid marketplace identity cell"
    );
    Ok(String::from_utf8(bytes)?)
}

fn marketplace(table: Table) -> Result<BTreeMap<u32, MarketItem>> {
    ensure!(
        table.found != Some(false)
            && table.columns == 12
            && (1..=16666).contains(&table.rows)
            && table.cells_base64.len() == table.rows * table.columns,
        "incomplete target marketplace table"
    );
    let mut items = BTreeMap::new();
    for cells in table.cells_base64.chunks_exact(12) {
        let id = decode_cell(&cells[0])?
            .parse::<u32>()
            .context("invalid marketplace item ID")?;
        let name = decode_cell(&cells[1])?;
        let category = decode_cell(&cells[2])?;
        ensure!(
            (1..=65535).contains(&id) && identifier(&name),
            "invalid marketplace item identity"
        );
        ensure!(
            items.insert(id, MarketItem { name, category }).is_none(),
            "duplicate marketplace item ID {id}"
        );
    }
    Ok(items)
}

fn valid_part(part: &Part) -> bool {
    (1..=65535).contains(&part.item_id)
        && part.item_id != 20008
        && part.kind < 16
        && (1..0x3fff).contains(&part.wire_index)
        && identifier(&part.name)
}

fn canonical_part(part: &Part, market: &BTreeMap<u32, MarketItem>) -> bool {
    market
        .get(&part.item_id)
        .is_some_and(|item| item.category == "wat" && item.name == part.name)
}

impl Arsenal {
    pub(crate) fn load(path: impl AsRef<Path>, expected_crc: u32) -> Result<Self> {
        let path = path.as_ref();
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .with_context(|| format!("read arsenal {}", path.display()))?
            .take(MAX_CAPTURE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes, expected_crc)
            .with_context(|| format!("invalid arsenal {}", path.display()))
    }

    fn from_bytes(bytes: &[u8], expected_crc: u32) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_CAPTURE_BYTES,
            "arsenal capture size limit"
        );
        let capture: Capture = serde_json::from_slice(bytes).context("decode arsenal capture")?;
        ensure!(
            capture.schema_version == 1
                && capture.read_only
                && capture.client_sha256 == codol_protocol::CLIENT_SHA256
                && capture.connection_state == 10
                && capture.ncs_crc == expected_crc
                && (1..900).contains(&capture.registered_count),
            "arsenal client/state/net-constant identity mismatch"
        );
        ensure!(
            !capture.weapons.is_empty()
                && capture.weapons.len() <= usize::from(capture.registered_count),
            "arsenal weapon count invalid"
        );
        let market = marketplace(capture.marketplace)?;
        let mut weapons = BTreeMap::new();
        let mut item_to_base = BTreeMap::new();
        let mut weapon_names = BTreeSet::new();
        let mut registry_names = BTreeMap::new();
        for captured in capture.weapons {
            ensure!(
                captured
                    .melee_charge_animation
                    .as_ref()
                    .is_none_or(|name| name.is_empty() || identifier(name)),
                "invalid captured melee charge animation"
            );
            let melee_charge_animation = captured
                .melee_charge_animation
                .as_ref()
                .is_some_and(|name| !name.is_empty());
            let captured_defaults = match captured.defaults {
                Some(defaults) if !captured.alternate_only => defaults,
                None if captured.alternate_only => Defaults {
                    weapon: captured.name.clone(),
                    parts: Vec::new(),
                },
                _ => anyhow::bail!("inconsistent alternate weapon catalogue"),
            };
            ensure!(
                identifier(&captured.name)
                    && (1..=capture.registered_count).contains(&captured.base_index)
                    && weapon_names.insert(captured.name.clone())
                    && captured_defaults.weapon == captured.name,
                "weapon registry identity mismatch: {}",
                captured.name
            );
            ensure!(
                (captured.alternate_only == captured.item_ids.is_empty())
                    && captured.item_ids.len() <= market.len()
                    && captured_defaults.parts.len() <= 16
                    && captured.compatible.len() <= 64,
                "weapon catalogue limits: {}",
                captured.name
            );
            for id in captured.item_ids {
                ensure!(
                    market
                        .get(&id)
                        .is_some_and(|item| item.category == "wea" && item.name == captured.name),
                    "weapon item {id} does not match independent marketplace"
                );
                ensure!(
                    item_to_base.insert(id, captured.base_index).is_none(),
                    "duplicate weapon item {id}"
                );
            }
            let definition = STANDARD
                .decode(captured.definition_base64)
                .context("invalid WeaponDef base64")?;
            ensure!(
                definition.len() == crate::weapon_attributes::WEAPON_DEF_SIZE,
                "weapon {} has invalid definition length",
                captured.name
            );
            let mut defaults = BTreeMap::new();
            let mut items = BTreeMap::new();
            let mut attributes = BTreeMap::new();
            let mut part_wires = BTreeMap::new();
            for part in captured_defaults.parts {
                ensure!(
                    valid_part(&part) && canonical_part(&part, &market),
                    "invalid default part {} for {}",
                    part.item_id,
                    captured.name
                );
                ensure!(
                    defaults.insert(part.kind, part.clone()).is_none(),
                    "duplicate default attachment kind"
                );
                ensure!(
                    part_wires
                        .insert(part.wire_index, (part.name.clone(), part.kind))
                        .is_none(),
                    "duplicate default attachment wire index"
                );
                items.insert(part.item_id, part);
            }
            for compatible in captured.compatible {
                ensure!(
                    identifier(&compatible.name)
                        && (1..0x3fff).contains(&compatible.wire_index)
                        && compatible.items.len() <= 65535,
                    "invalid compatible attachment identity"
                );
                if let Some(previous) =
                    registry_names.insert(compatible.wire_index, compatible.name.clone())
                {
                    ensure!(
                        previous == compatible.name,
                        "attachment registry index has conflicting names"
                    );
                }
                let bytes = STANDARD
                    .decode(compatible.attributes_base64)
                    .context("invalid attribute record base64")?;
                ensure!(
                    bytes.len() % 12 == 0 && bytes.len() <= 256 * 12,
                    "invalid attachment record array"
                );
                let records: Vec<[u8; 12]> = bytes
                    .chunks_exact(12)
                    .map(|r| r.try_into().unwrap())
                    .collect();
                // 42e3b0 walks the weapon's (wire, attribute) pairs forwards
                // and exits at the first match. Preserve that selection.
                attributes.entry(compatible.wire_index).or_insert(records);
                for part in compatible.items {
                    if matches!(part.item_id, 0 | 20008) {
                        continue;
                    }
                    ensure!(
                        valid_part(&part)
                            && part.name == compatible.name
                            && part.wire_index == compatible.wire_index,
                        "compatible attachment record identity mismatch"
                    );
                    // Compact lookup aliases absent from the independent
                    // item table must never become ownable selections.
                    if !canonical_part(&part, &market) {
                        continue;
                    }
                    if let Some(previous) =
                        part_wires.insert(part.wire_index, (part.name.clone(), part.kind))
                    {
                        ensure!(
                            previous == (part.name.clone(), part.kind),
                            "attachment wire has conflicting kinds"
                        );
                    }
                    if let Some(previous) = items.insert(part.item_id, part.clone()) {
                        ensure!(
                            previous == part,
                            "attachment item maps to conflicting parts"
                        );
                    }
                }
            }
            for part in defaults.values() {
                if let Some(previous) = registry_names.insert(part.wire_index, part.name.clone()) {
                    ensure!(
                        previous == part.name,
                        "default attachment registry identity mismatch"
                    );
                }
            }
            ensure!(
                weapons
                    .insert(
                        captured.base_index,
                        Weapon {
                            name: captured.name,
                            definition,
                            melee_charge_animation,
                            defaults,
                            items,
                            attributes
                        }
                    )
                    .is_none(),
                "duplicate base weapon registry index"
            );
        }
        Ok(Self {
            source_sha256: format!("{:x}", Sha256::digest(bytes)),
            registered_count: capture.registered_count,
            weapons,
            item_to_base,
        })
    }

    pub(crate) fn next_index(&self) -> u16 {
        self.registered_count + 1
    }
    pub(crate) fn bot_rifle(&self, index: u16) -> Result<ResolvedWeapon> {
        self.bot_weapon("m4_mp", index)
    }
    /// Bot loadouts derive from captured class names, not a player's inventory.
    /// Defaults and per-weapon attributes still pass through the normal resolver.
    pub(crate) fn bot_weapon(&self, name: &str, index: u16) -> Result<ResolvedWeapon> {
        let (&item, _) = self
            .item_to_base
            .iter()
            .find(|(_, base)| self.weapons[base].name == name)
            .ok_or_else(|| anyhow::anyhow!("native bot weapon {name} absent from arsenal"))?;
        self.resolve(item, &[], &[item], index)
    }

    /// Alternate ownership derives from the already validated parent. Apply
    /// the same model parts using the alternate's own attribute table.
    pub(crate) fn resolve_alternate(
        &self,
        parent: &ResolvedWeapon,
    ) -> Result<Option<ResolvedWeapon>> {
        let base = u32::from_le_bytes(parent.definition[0xd24..0xd28].try_into().unwrap());
        if base == 0 {
            return Ok(None);
        }
        let base = u16::try_from(base).context("invalid alternate base")?;
        let weapon = self
            .weapons
            .get(&base)
            .ok_or_else(|| anyhow::anyhow!("alternate definition {base} was not captured"))?;
        let records: Vec<_> = parent
            .attachments
            .iter()
            .filter_map(|wire| weapon.attributes.get(wire))
            .flatten()
            .copied()
            .collect();
        Ok(Some(ResolvedWeapon {
            name: weapon.name.clone(),
            base_index: base,
            index: parent.index + 1,
            attachments: parent.attachments.clone(),
            composed_name: codol_protocol::game_state::composed_weapon(
                base,
                0,
                &parent.attachments,
            )?,
            definition: crate::weapon_attributes::compose(&weapon.definition, &records)?,
            melee_charge_animation: weapon.melee_charge_animation,
        }))
    }

    pub(crate) fn resolve(
        &self,
        item_id: u32,
        attachment_ids: &[u32],
        owned: &[u32],
        wire_index: u16,
    ) -> Result<ResolvedWeapon> {
        let base_index = *self.item_to_base.get(&item_id).ok_or_else(|| {
            anyhow::anyhow!("backpack weapon item {item_id} is absent from captured arsenal")
        })?;
        ensure!(
            owned.contains(&item_id),
            "player does not own backpack weapon {item_id}"
        );
        // registered_count is a live diagnostic and may include previously
        // composed weapons. The connection owns its canonical-name registry
        // and allocates slots from the independently prepared NCS base count.
        ensure!(
            base_index < wire_index && wire_index < 900,
            "composed weapon index must follow its base and remain below 900 (capture next index {})",
            self.next_index()
        );
        ensure!(attachment_ids.len() <= 16, "too many backpack attachments");
        let weapon = &self.weapons[&base_index];
        let mut selected: BTreeMap<_, _> = weapon
            .defaults
            .iter()
            .map(|(&kind, part)| (kind, part))
            .collect();
        let mut selected_kinds = BTreeSet::new();
        for &id in attachment_ids {
            if matches!(id, 0 | 20008) {
                continue;
            }
            let part = weapon.items.get(&id).ok_or_else(|| {
                anyhow::anyhow!(
                    "backpack attachment {id} is not a marketplace-compatible part of {}",
                    weapon.name
                )
            })?;
            ensure!(
                owned.contains(&id),
                "player does not own backpack attachment {id}"
            );
            ensure!(
                selected_kinds.insert(part.kind),
                "backpack has conflicting attachment kind {}",
                part.kind
            );
            selected.insert(part.kind, part);
        }
        let attachments: Vec<_> = selected.values().map(|part| part.wire_index).collect();
        let mut records = Vec::new();
        for part in selected.values() {
            if let Some(attributes) = weapon.attributes.get(&part.wire_index) {
                records.extend_from_slice(attributes);
            }
            // A default lacking a per-weapon attribute pair resolves to the
            // target fallback asset (no fields), while remaining a model part.
        }
        let definition = crate::weapon_attributes::compose(&weapon.definition, &records)
            .with_context(|| format!("compose {} attachment properties", weapon.name))?;
        let composed_name =
            codol_protocol::game_state::composed_weapon(base_index, 0, &attachments)
                .context("encode target composed weapon name")?;
        Ok(ResolvedWeapon {
            name: weapon.name.clone(),
            base_index,
            index: wire_index,
            attachments,
            composed_name,
            definition,
            melee_charge_animation: weapon.melee_charge_animation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn part(item_id: u32, kind: u8, name: &str, wire_index: u16) -> Value {
        json!({"item_id":item_id,"kind":kind,"name":name,"wire_index":wire_index})
    }

    fn attributes(offset: u16, value: f32) -> String {
        let mut record = [0u8; 12];
        record[0] = 4;
        record[1] = 6;
        record[4..6].copy_from_slice(&offset.to_le_bytes());
        record[8..].copy_from_slice(&value.to_bits().to_le_bytes());
        STANDARD.encode(record)
    }

    fn capture() -> Value {
        let rows = [
            (10001, "m4_mp", "wea"),
            (10010, "pistol_mp", "wea"),
            (20032, "bar_default", "wat"),
            (20031, "mag_default", "wat"),
            (20659, "bar_custom", "wat"),
            (21237, "mag_custom", "wat"),
            (20999, "bar_custom", "wat"),
            (21999, "another_part", "wat"),
        ];
        let mut cells = Vec::new();
        for (id, name, category) in rows {
            for cell in [id.to_string(), name.into(), category.into()]
                .into_iter()
                .chain(std::iter::repeat_n(String::new(), 9))
            {
                cells.push(STANDARD.encode(cell));
            }
        }
        let mut definition = vec![0u8; crate::weapon_attributes::WEAPON_DEF_SIZE];
        definition[0xd08..0xd0c].copy_from_slice(&20u32.to_le_bytes());
        let default_bar = part(20032, 4, "bar_default", 10);
        let default_mag = part(20031, 9, "mag_default", 11);
        json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "connection_state":10,"ncs_crc":123,"registered_count":351,
            "marketplace":{"found":true,"name":"mp/coMarketplaceItems.csv","columns":12,"rows":rows.len(),"cells_base64":cells},
            "weapons":[
                {"name":"m4_mp","base_index":163,"item_ids":[10001],"definition_base64":STANDARD.encode(&definition),
                 "defaults":{"weapon":"m4_mp","parts":[default_bar.clone(),default_mag]},
                 "compatible":[
                    {"name":"bar_default","wire_index":10,"items":[default_bar],"attributes_base64":attributes(0xd10,80.0)},
                    {"name":"bar_custom","wire_index":20,"items":[part(20659,4,"bar_custom",20),part(20999,4,"bar_custom",20),part(21999,4,"bar_custom",20),part(9999,4,"bar_custom",20)],"attributes_base64":attributes(0xd10,60.0)},
                    {"name":"mag_custom","wire_index":21,"items":[part(21237,9,"mag_custom",21)],"attributes_base64":attributes(0xd08,45.0)}
                 ]},
                {"name":"pistol_mp","base_index":150,"item_ids":[10010],"definition_base64":STANDARD.encode(&definition),
                 "defaults":{"weapon":"pistol_mp","parts":[]},"compatible":[]}
            ]
        })
    }

    fn load(value: &Value) -> Arsenal {
        Arsenal::from_bytes(&serde_json::to_vec(value).unwrap(), 123).unwrap()
    }

    fn word(value: &ResolvedWeapon, offset: usize) -> u32 {
        u32::from_le_bytes(value.definition[offset..offset + 4].try_into().unwrap())
    }

    #[test]
    fn alternate_definitions_use_parent_parts_without_becoming_ownable_items() {
        let mut value = capture();
        let mut definition = STANDARD
            .decode(value["weapons"][0]["definition_base64"].as_str().unwrap())
            .unwrap();
        definition[0xd24..0xd28].copy_from_slice(&31u32.to_le_bytes());
        value["weapons"][0]["definition_base64"] = json!(STANDARD.encode(&definition));
        definition[0xd24..0xd28].fill(0);
        value["weapons"].as_array_mut().unwrap().push(json!({
            "name":"altfire_mp", "base_index":31, "alternate_only":true,
            "definition_base64":STANDARD.encode(definition),
            "compatible":[{"name":"bar_custom","wire_index":20,"items":[],"attributes_base64":attributes(0xd10,400.0)}]
        }));
        let arsenal = load(&value);
        let parent = arsenal
            .resolve(10001, &[20659], &[10001, 20659], 352)
            .unwrap();
        let alternate = arsenal.resolve_alternate(&parent).unwrap().unwrap();
        assert_eq!(alternate.attachments, parent.attachments);
        assert_eq!(word(&alternate, 0xd10), 400);
        assert!(arsenal.resolve(31, &[], &[31], 355).is_err());
        value["weapons"].as_array_mut().unwrap().pop();
        assert!(load(&value).resolve_alternate(&parent).is_err());
    }

    #[test]
    fn resolves_independent_players_weapons_and_unenumerated_combinations() {
        let arsenal = load(&capture());
        assert_eq!(arsenal.next_index(), 352);
        let a = arsenal
            .resolve(
                10001,
                &[21237, 20659, 0, 20008],
                &[10001, 21237, 20659],
                352,
            )
            .unwrap();
        assert_eq!(
            (a.name.as_str(), a.base_index, a.index),
            ("m4_mp", 163, 352)
        );
        assert_eq!(a.attachments, vec![20, 21]);
        assert_eq!(a.composed_name, "** 163 0 0 20 21");
        assert_eq!(word(&a, 0xd08), 45);
        assert_eq!(word(&a, 0xd10), 60);
        let default = arsenal.resolve(10001, &[], &[10001], 352).unwrap();
        assert_eq!(default.attachments, vec![10, 11]);
        assert_eq!(word(&default, 0xd08), 20);
        assert_eq!(word(&default, 0xd10), 80);
        let mag_only = arsenal
            .resolve(10001, &[21237], &[10001, 21237], 352)
            .unwrap();
        assert_eq!(mag_only.attachments, vec![10, 21]);
        assert_eq!(word(&mag_only, 0xd08), 45);
        let pistol = arsenal.resolve(10010, &[], &[10010], 353).unwrap();
        assert_eq!(pistol.composed_name, "** 150 0 0");
    }
    #[test]
    fn captured_charge_animation_belongs_to_the_selected_weapon() {
        let mut data = capture();
        data["weapons"][0]["melee_charge_animation"] = json!("viewmodel_knife_melee_stab");
        data["weapons"][1]["melee_charge_animation"] = json!("");
        let a = load(&data);
        assert!(
            a.resolve(10001, &[], &[10001], 352)
                .unwrap()
                .melee_charge_animation
        );
        assert!(
            !a.resolve(10010, &[], &[10010], 353)
                .unwrap()
                .melee_charge_animation
        );
        data["weapons"][0]["melee_charge_animation"] = json!("bad\nname");
        assert!(Arsenal::from_bytes(&serde_json::to_vec(&data).unwrap(), 123).is_err());
    }

    #[test]
    fn independent_marketplace_filters_compact_registry_aliases() {
        let arsenal = load(&capture());
        for alias in [9999, 21999] {
            assert!(
                arsenal
                    .resolve(10001, &[alias], &[10001, alias], 352)
                    .is_err()
            );
        }
        // A second actual marketplace ID for the same named part is valid.
        assert_eq!(
            arsenal
                .resolve(10001, &[20999], &[10001, 20999], 352)
                .unwrap()
                .attachments,
            vec![20, 11]
        );
    }

    #[test]
    fn rejects_unowned_unknown_conflicting_and_wrong_weapon_selections() {
        let arsenal = load(&capture());
        assert!(arsenal.resolve(10001, &[], &[], 352).is_err());
        assert!(arsenal.resolve(10001, &[20659], &[10001], 352).is_err());
        assert!(arsenal.resolve(99999, &[], &[99999], 352).is_err());
        assert!(
            arsenal
                .resolve(10010, &[20659], &[10010, 20659], 352)
                .is_err()
        );
        assert!(
            arsenal
                .resolve(10001, &[20659, 20999], &[10001, 20659, 20999], 352)
                .is_err()
        );
        assert!(arsenal.resolve(10001, &[], &[10001], 163).is_err());
        assert!(arsenal.resolve(10001, &[], &[10001], 900).is_err());
        // A prior diagnostic composition in the capture does not reserve a
        // slot in a newly initialized client.
        assert!(arsenal.resolve(10001, &[], &[10001], 351).is_ok());
    }

    #[test]
    fn first_registered_attribute_mapping_wins_without_repeating_effects() {
        let mut data = capture();
        let entries = data["weapons"][0]["compatible"].as_array_mut().unwrap();
        let mut duplicate = entries[1].clone();
        duplicate["attributes_base64"] = json!(attributes(0xd10, 5.0));
        entries.push(duplicate);
        let resolved = load(&data)
            .resolve(10001, &[20659], &[10001, 20659], 352)
            .unwrap();
        assert_eq!(word(&resolved, 0xd10), 60);
    }

    #[test]
    fn checks_capture_identity_shapes_and_preserves_evidence_hash() {
        let data = capture();
        let bytes = serde_json::to_vec(&data).unwrap();
        assert_eq!(
            load(&data).source_sha256,
            format!("{:x}", Sha256::digest(&bytes))
        );
        assert!(Arsenal::from_bytes(&bytes, 124).is_err());
        for (path, value) in [
            ("/read_only", json!(false)),
            ("/connection_state", json!(9)),
            ("/client_sha256", json!("a".repeat(64))),
            ("/weapons/0/item_ids", json!([10010])),
            (
                "/weapons/0/definition_base64",
                json!(STANDARD.encode([0; 4])),
            ),
            (
                "/weapons/0/compatible/0/attributes_base64",
                json!(STANDARD.encode([0; 13])),
            ),
            ("/weapons/0/defaults/parts/0/name", json!("wrong")),
            ("/marketplace/rows", json!(99)),
        ] {
            let mut bad = data.clone();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(
                Arsenal::from_bytes(&serde_json::to_vec(&bad).unwrap(), 123).is_err(),
                "{path}"
            );
        }
    }
}
