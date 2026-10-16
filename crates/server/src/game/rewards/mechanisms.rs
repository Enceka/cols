//! Native identities grouped by the controller they need. A registered
//! activator is independent of whether that controller is available.
use super::killstreaks::RadarResources;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mechanism {
    Radar,
    Airdrop,
    LocationStrike,
    Remote,
    AttackAircraft,
    Emp,
    Nuclear,
    Deployable,
    Dogs,
    Armor,
    Drone,
    Mode,
}
impl Mechanism {
    pub fn for_reward(id: u8) -> Option<Self> {
        Some(match id {
            1 | 3 | 19 => Self::Radar,
            2 | 4 | 9 | 22 => Self::Airdrop,
            6 | 7 | 11 | 17 => Self::LocationStrike,
            5 | 12 | 13 | 18 => Self::Remote,
            8 | 10 => Self::AttackAircraft,
            14 => Self::Emp,
            15 => Self::Nuclear,
            16 | 21 => Self::Deployable,
            20 => Self::Dogs,
            23 | 24 => Self::Armor,
            26 => Self::Drone,
            25 | 27 | 50 => Self::Mode,
            _ => return None,
        })
    }
    pub fn marker(id: u8) -> bool {
        Self::for_reward(id) == Some(Self::Airdrop)
    }
    pub fn device(id: u8) -> bool {
        Self::for_reward(id) == Some(Self::Deployable)
    }
    pub fn radio_available(id: u8, resources: &RadarResources) -> bool {
        match Self::for_reward(id) {
            Some(Self::Radar | Self::Emp) => true,
            Some(Self::Deployable) => {
                if id == 21 {
                    super::deployables::DeviceDefinition::sam_from_resources(resources).is_some()
                } else {
                    super::deployables::DeviceDefinition::from_resources(resources).is_some()
                }
            }
            Some(Self::Airdrop) if matches!(id, 4 | 22) => {
                super::airdrops::Airdrops::resources_present(resources, id)
                    && Self::radio_available(if id == 4 { 16 } else { 21 }, resources)
            }
            Some(Self::LocationStrike) if id != 7 => {
                super::strikes::Strikes::resources_present(resources, id)
            }
            // The runtime exposes these experimental controllers when the
            // map has their bound resources. Availability is not a stability claim.
            Some(Self::Remote) if id == 5 => {
                resources
                    .weapons
                    .contains_key("remotemissile_projectile_mp")
                    && resources.models.contains_key("projectile_hellfire_missile")
                    && resources.tags.contains_key("tag_origin")
                    && resources.remote_spawns.is_some()
            }
            Some(Self::AttackAircraft) if id == 8 => {
                resources.weapons.contains_key("cobra_20mm_mp")
                    && resources.models.contains_key("vehicle_mi24p_hind_mp")
                    && resources.model_bounds.contains_key("vehicle_mi24p_hind_mp")
                    && resources.vehicles.contains_key("cobra_mp")
                    && resources.animations.contains_key("bh_rotors")
                    && resources.air_paths.is_some()
            }
            // The complete random airdrop pool includes the controllers below.
            // Geometry and pickup tests do not establish native usability.
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_catalog_identities_have_a_group_without_aliasing_pro_or_modes() {
        for id in (1..=27).chain([50]) {
            assert!(
                Mechanism::for_reward(id).is_some(),
                "ungrouped native reward {id}"
            );
        }
        for id in [0, 28, 43, 49, 51, 255] {
            assert!(Mechanism::for_reward(id).is_none());
        }
        for id in [2, 4, 9, 22] {
            assert!(Mechanism::marker(id));
        }
        for id in [5, 12, 13, 18] {
            assert_eq!(Mechanism::for_reward(id), Some(Mechanism::Remote));
        }
        for id in [6, 7, 11, 17] {
            assert_eq!(Mechanism::for_reward(id), Some(Mechanism::LocationStrike));
        }
        assert_eq!(Mechanism::for_reward(23), Some(Mechanism::Armor));
        assert_eq!(Mechanism::for_reward(24), Some(Mechanism::Armor));
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires CODOL_REWARD_CATALOG and the prepared local maps"]
fn prepared_catalog_exposes_experimental_rewards_on_each_map() {
    let catalog = std::path::PathBuf::from(std::env::var_os("CODOL_REWARD_CATALOG").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&catalog).unwrap()).unwrap();
    let entries = manifest["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    let mut failures = Vec::new();
    for entry in entries {
        let path = catalog
            .parent()
            .unwrap()
            .join(entry["file"].as_str().unwrap());
        let state = crate::game::session::prepared::PreparedGameState::load(&path).unwrap();
        let resources = state.radar_resources().expect("map reward resources");
        for id in [1, 3, 5, 8, 16] {
            assert!(
                resources.radio(id).is_some() && Mechanism::radio_available(id, resources),
                "{} / {} lacks reward {id}",
                entry["map"],
                entry["mode"]
            );
        }
        assert!(state.nuke_resources().is_some(), "map nuclear resources");
        let collision = state.collision.as_ref().unwrap();
        let player = state.local_player_snapshot(true).unwrap();
        let missile = super::remote::Remotes::default()
            .plan(resources, player.origin, player.angles, &[], collision)
            .is_some();
        let helicopter = super::aircraft::Aircraft::default()
            .plan(resources, 8, collision)
            .is_some();
        eprintln!(
            "{} / {}: predator_plan={missile}, helicopter_plan={helicopter}",
            entry["map"], entry["mode"]
        );
        if !missile || !helicopter {
            failures.push(serde_json::json!({"map":entry["map"],"mode":entry["mode"],"predator":missile,"helicopter":helicopter}));
        }
    }
    if let Some(path) = std::env::var_os("CODOL_REWARD_PLAN_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&failures).unwrap()).unwrap();
    }
    assert!(
        failures.is_empty(),
        "experimental map plans failed: {failures:?}"
    );
}
