pub(crate) mod aircraft;
pub(crate) mod airdrops;
pub(crate) mod airspace;
pub(crate) mod armor;
pub(crate) mod deployables;
pub(crate) mod dogs;
pub(crate) mod drones;
mod gunners;
#[cfg(test)]
pub(crate) use gunners::tests::resources as gunner_test_resources;
pub(crate) mod inventory;
pub(crate) mod killstreaks;
pub(crate) mod mechanisms;
pub(crate) mod paths;
pub(crate) mod remote;
mod remote_car;
pub(crate) mod sam;
pub(crate) mod scavenger;
pub(crate) mod selection;
pub(crate) mod strikes;

#[cfg(test)]
mod paths_tests;
#[cfg(test)]
pub(crate) mod strikes_tests;
