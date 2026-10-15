//! Native map IDs and allowed TDM identities; assets still require preparation.
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    table: String,
    table_cells_sha256: String,
    resource_preparation_claimed: bool,
    maps: Vec<Map>,
}
#[derive(Deserialize)]
struct Map {
    name: String,
    native_id: u32,
    modes: Vec<String>,
    maximum_players: u8,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let c: Catalog = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/map-registry-3.3.3.3.json"
    )))
    .expect("captured native map identities");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.capture_sha256.len(), 64);
    assert_eq!(c.table, "mp/coMapList.csv");
    assert_eq!(c.table_cells_sha256.len(), 64);
    assert!(!c.resource_preparation_claimed);
    assert_eq!(c.maps.len(), 26);
    let mut names = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for map in &c.maps {
        assert!(names.insert(&map.name) && ids.insert(map.native_id));
        assert!((1..=512).contains(&map.native_id));
        assert!(map.modes.iter().any(|mode| mode == "war"));
    }
    c
});
static CHARGE: LazyLock<Catalog> = LazyLock::new(|| {
    let c: Catalog = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/charge-map-registry-3.3.3.3.json"
    )))
    .expect("captured native charge maps");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.table, "mp/coMapList.csv");
    assert_eq!(c.capture_sha256.len(), 64);
    assert_eq!(c.table_cells_sha256.len(), 64);
    assert!(!c.resource_preparation_claimed);
    assert_eq!(c.maps.len(), 17);
    let mut names = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for map in &c.maps {
        assert!(names.insert(&map.name) && ids.insert(map.native_id));
        assert!((1..=512).contains(&map.native_id));
        assert!(map.modes.iter().any(|m| m == "cha"));
        if let Some(shared) = CATALOG.maps.iter().find(|m| m.name == map.name) {
            assert_eq!(shared.native_id, map.native_id);
            assert_eq!(shared.maximum_players, map.maximum_players);
        }
    }
    c
});
pub(crate) fn native_id(name: &str) -> Option<u32> {
    CATALOG
        .maps
        .iter()
        .chain(CHARGE.maps.iter())
        .find(|map| map.name == name)
        .map(|map| map.native_id)
}
pub(crate) fn maximum_players(name: &str) -> Option<u8> {
    CATALOG
        .maps
        .iter()
        .chain(CHARGE.maps.iter())
        .find(|map| map.name == name)
        .map(|map| map.maximum_players)
}
pub(crate) fn supports_mode(name: &str, mode: &str) -> bool {
    matches!(mode, "war" | "cha")
        && CATALOG
            .maps
            .iter()
            .chain(CHARGE.maps.iter())
            .any(|map| map.name == name && map.modes.iter().any(|m| m == mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_ids_include_noncontiguous_and_non_mp_entries_without_enabling_unknown_names() {
        for (name, id) in [
            ("mp_afghan", 1),
            ("mp_firingrange", 15),
            ("mp_seatown", 63),
            ("za_coliseum", 83),
        ] {
            assert_eq!(native_id(name), Some(id));
        }
        for name in [
            "mp_unknown",
            "mp_seatown;quit",
            "../mp_afghan",
            "coop_river",
            "MP_AFGHAN",
        ] {
            assert_eq!(native_id(name), None);
        }
    }
}
