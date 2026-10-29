//! Hand a finished cooperative mission to one persistent completion record.
use super::*;

impl Connection {
    /// The mission reports one completion; the server records it once and
    /// retries on failure, so a lost reply never hides the unlock.
    pub(crate) fn pending_story_completion(&self) -> Option<(String, u32, u32)> {
        if self.story_committed
            || self
                .story_retry_at
                .is_some_and(|next| Instant::now() < next)
        {
            return None;
        }
        self.story_completion.clone()
    }
    pub(crate) fn story_completion_failed(&mut self) {
        self.story_retry_at = Some(Instant::now() + Duration::from_secs(1));
    }
    pub(crate) fn story_completion_committed(&mut self) {
        self.story_committed = true;
        self.story_retry_at = None;
    }
}
