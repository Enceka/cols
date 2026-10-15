//! Frozen authenticated membership for one room's game allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Participant {
    pub user_id: u64,
    pub seed: u32,
    /// Game team: 1 Axis, 2 Allies.
    pub team: u8,
    pub bot: Option<u8>,
}
