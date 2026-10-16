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
