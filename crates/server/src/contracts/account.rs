//! Stable account identity without login credentials or transport state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccountId {
    pub title_id: u32,
    pub user_id: u64,
}
