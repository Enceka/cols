//! Composed weapon identities persist when the client exits and re-enters a map.
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct WeaponRegistry {
    pub names: BTreeMap<String, u16>,
    pub next: u16,
}
impl WeaponRegistry {
    pub fn new(first: u16) -> Self {
        Self {
            names: BTreeMap::new(),
            next: first,
        }
    }
    pub fn configstrings(&self) -> Vec<(u16, String)> {
        let mut values = self
            .names
            .iter()
            .map(|(name, &id)| (5443 + id, name.clone()))
            .collect::<Vec<_>>();
        values.sort_by_key(|v| v.0);
        values
    }
}
