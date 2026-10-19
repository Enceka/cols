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
        if resources.radio(id).is_none() {
            return false;
        }
        match Self::for_reward(id) {
            Some(Self::Radar) if matches!(id, 1 | 3) => {
                super::airspace::Airspace::resources_present(resources, id)
            }
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
            Some(Self::Airdrop) if matches!(id, 2 | 9) => {
                super::airdrops::Airdrops::resources_present(resources, id)
                    && super::airdrops::Airdrops::random_rewards(id).is_some_and(|pool| {
                        pool.iter().all(|(contents, _)| match contents {
                            super::airdrops::Contents::Ammo => true,
                            super::airdrops::Contents::Reward(child) => {
                                Self::radio_available(*child, resources)
                            }
                        })
                    })
            }
            Some(Self::LocationStrike) if id == 7 => {
                super::strikes::Strikes::resources_present(resources, id)
                    && super::aircraft::Aircraft::harrier_resources_present(resources)
            }
            Some(Self::LocationStrike) => super::strikes::Strikes::resources_present(resources, id),
            // The runtime exposes these experimental controllers when the
            // map has their bound resources. Availability is not a stability claim.
            Some(Self::Remote) if id == 5 => super::remote::Remotes::resources_present(resources),
            Some(Self::Remote) if matches!(id, 12 | 13) => {
                super::gunners::Gunners::resources_present(resources, id)
            }
            Some(Self::Remote) if id == 18 => {
                super::remote_car::Car::resources_present(resources)
                    && resources.tags.contains_key("tag_player")
            }
            Some(Self::AttackAircraft) if id == 8 => {
                super::aircraft::Aircraft::resources_present(resources, id)
            }
            Some(Self::AttackAircraft) if id == 10 => {
                super::aircraft::Aircraft::resources_present(resources, id)
            }
            Some(Self::Dogs) => super::dogs::Dogs::resources_present(resources),
            Some(Self::Armor) => super::armor::Plan::resources_present(resources, id),
            Some(Self::Drone) => super::drones::Drones::resources_present(resources),
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

    #[test]
    fn random_airdrop_availability_covers_its_exact_outcome_pool() {
        let ordinary = super::super::airdrops::Airdrops::random_rewards(2).unwrap();
        let mega = super::super::airdrops::Airdrops::random_rewards(9).unwrap();
        let rewards = |pool: &[(super::super::airdrops::Contents, u32)]| {
            pool.iter()
                .filter_map(|(content, _)| match content {
                    super::super::airdrops::Contents::Reward(id) => Some(*id),
                    super::super::airdrops::Contents::Ammo => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rewards(ordinary),
            [1, 3, 16, 5, 6, 7, 8, 10, 11, 12, 13, 14]
        );
        assert_eq!(rewards(mega), [1, 3, 16, 5, 6, 7, 8, 10, 11, 12, 13]);
        assert_eq!(ordinary.iter().map(|(_, weight)| weight).sum::<u32>(), 115);
        assert_eq!(mega.iter().map(|(_, weight)| weight).sum::<u32>(), 100);
        assert!(super::super::airdrops::Airdrops::random_rewards(4).is_none());
    }

    #[test]
    fn radar_support_is_not_advertised_without_its_aircraft_assets() {
        let mut resources = super::super::airspace::test_resources();
        for id in [1, 3] {
            assert!(Mechanism::radio_available(id, &resources));
        }
        resources.models.remove("vehicle_uav_static02_mp");
        assert!(!Mechanism::radio_available(1, &resources));
        assert!(Mechanism::radio_available(3, &resources));
    }

    #[test]
    fn mode_specific_identities_never_borrow_a_base_effect_handler() {
        let mut resources = super::super::remote::test_resources();
        let mut radio = resources.radios[0].clone();
        radio.native_id = 27;
        radio.weapon = 313;
        resources.radios.push(radio);
        assert!(!Mechanism::radio_available(25, &resources));
        assert!(!Mechanism::radio_available(27, &resources));
        assert!(!Mechanism::radio_available(50, &resources));
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
