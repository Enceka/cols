//! Stable reward instances follow the AlphaBuild ownership/use boundary.
//! HUD slots are presentation; successful use consumes one exact instance.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_INVENTORY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    inventory: u64,
    serial: u64,
    attempt: u64,
    owner: u8,
    native_id: u8,
    use_life: u32,
}
#[derive(Clone, Copy)]
struct Stock {
    serial: u64,
    owner: u8,
    native_id: u8,
    earned_life: u32,
}
pub(crate) struct Inventory {
    identity: u64,
    serial: u64,
    use_serial: u64,
    stock: Vec<Stock>,
    award_life: u32,
    natural_awards: BTreeSet<u8>,
    pending: Option<Ticket>,
}
impl Ticket {
    pub fn native_id(self) -> u8 {
        self.native_id
    }
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            identity: NEXT_INVENTORY
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
                .expect("inventory identity exhausted"),
            serial: 0,
            use_serial: 0,
            stock: Vec::new(),
            award_life: 0,
            natural_awards: BTreeSet::new(),
            pending: None,
        }
    }
}
impl Inventory {
    pub fn owned_ids(&self) -> Vec<u8> {
        let mut ids = Vec::new();
        for s in self.stock.iter().rev() {
            if !ids.contains(&s.native_id) {
                ids.push(s.native_id);
            }
        }
        ids
    }
    pub fn contains(&self, native_id: u8) -> bool {
        self.stock.iter().any(|s| s.native_id == native_id)
    }
    pub fn grant(&mut self, owner: u8, native_id: u8, earned_life: u32) -> bool {
        if owner >= 12 || native_id == 0 || self.contains(native_id) {
            return false;
        }
        let Some(serial) = self.serial.checked_add(1) else {
            return false;
        };
        self.serial = serial;
        self.stock.push(Stock {
            serial,
            owner,
            native_id,
            earned_life,
        });
        true
    }
    pub fn reset_natural_round(&mut self, life: u32) {
        self.award_life = life;
        self.natural_awards.clear();
    }
    pub fn grant_natural(&mut self, owner: u8, native_id: u8, life: u32) -> bool {
        if owner >= 12 || native_id == 0 {
            return false;
        }
        if self.award_life != life {
            self.reset_natural_round(life);
        }
        // Reaching a threshold consumes this round's award opportunity even
        // when the same skill is still owned. Using it later cannot backfill it.
        if !self.natural_awards.insert(native_id) {
            return false;
        }
        self.grant(owner, native_id, life)
    }
    pub fn reserve(&mut self, owner: u8, native_id: u8, use_life: u32) -> Option<Ticket> {
        if let Some(pending) = self.pending {
            return (pending.owner == owner
                && pending.native_id == native_id
                && pending.use_life == use_life)
                .then_some(pending);
        }
        let s =
            self.stock.iter().rev().find(|s| {
                s.owner == owner && s.native_id == native_id && s.earned_life <= use_life
            })?;
        let attempt = self.use_serial.checked_add(1)?;
        self.use_serial = attempt;
        let ticket = Ticket {
            inventory: self.identity,
            serial: s.serial,
            attempt,
            owner,
            native_id,
            use_life,
        };
        self.pending = Some(ticket);
        Some(ticket)
    }
    pub fn commit(&mut self, ticket: Ticket, owner: u8, life: u32, alive: bool) -> bool {
        if ticket.inventory != self.identity
            || !alive
            || ticket.owner != owner
            || ticket.use_life != life
            || self.pending != Some(ticket)
        {
            return false;
        }
        let Some(index) = self.stock.iter().position(|s| {
            s.serial == ticket.serial && s.owner == owner && s.native_id == ticket.native_id
        }) else {
            return false;
        };
        self.stock.remove(index);
        self.pending = None;
        true
    }
    pub fn cancel(&mut self, ticket: Ticket) -> bool {
        if self.pending != Some(ticket) {
            return false;
        }
        self.pending = None;
        true
    }
    pub fn invalidate_use(&mut self) {
        self.pending = None;
    }
    pub fn clear(&mut self) {
        self.stock.clear();
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural_rewards_repeat_by_round_without_stacking_and_commits_cannot_replay() {
        let mut i = Inventory::default();
        assert!(i.grant_natural(0, 1, 0));
        assert!(!i.grant_natural(0, 1, 0));
        assert!(!i.grant_natural(0, 1, 1));
        let ticket = i.reserve(0, 1, 1).unwrap();
        i.reset_natural_round(1);
        assert!(!i.grant_natural(0, 1, 1));
        assert!(i.commit(ticket, 0, 1, true));
        assert!(!i.commit(ticket, 0, 1, true));
        assert!(!i.contains(1));
        assert!(!i.grant_natural(0, 1, 1));
        i.reset_natural_round(1);
        assert!(i.grant_natural(0, 1, 1));
        let next = i.reserve(0, 1, 1).unwrap();
        assert_ne!(next.serial, ticket.serial);
        assert!(i.commit(next, 0, 1, true));
        assert!(!i.contains(1));
    }
    #[test]
    fn another_earned_reward_does_not_redirect_a_deferred_commit() {
        let mut i = Inventory::default();
        assert!(i.grant(0, 16, 0));
        let ticket = i.reserve(0, 16, 0).unwrap();
        assert!(i.grant(0, 21, 0));
        assert!(i.reserve(0, 21, 0).is_none());
        assert!(!i.commit(ticket, 1, 0, true));
        assert!(i.commit(ticket, 0, 0, true));
        assert!(!i.contains(16));
        assert!(i.contains(21));
    }
    #[test]
    fn an_old_match_ticket_cannot_consume_an_equivalent_new_match_reward() {
        let mut a = Inventory::default();
        let mut b = Inventory::default();
        assert!(a.grant(0, 1, 0));
        assert!(b.grant(0, 1, 0));
        let old = a.reserve(0, 1, 0).unwrap();
        let current = b.reserve(0, 1, 0).unwrap();
        assert!(!b.commit(old, 0, 0, true));
        assert!(b.contains(1));
        assert!(b.commit(current, 0, 0, true));
    }
    #[test]
    fn death_cancels_permission_and_retains_inventory_for_the_next_life() {
        let mut i = Inventory::default();
        assert!(i.grant(0, 2, 0));
        let old = i.reserve(0, 2, 0).unwrap();
        assert!(!i.commit(old, 0, 0, false));
        assert!(i.cancel(old));
        assert!(!i.cancel(old));
        i.invalidate_use();
        assert!(!i.commit(old, 0, 1, true));
        assert!(i.contains(2));
        let retry = i.reserve(0, 2, 1).unwrap();
        assert!(i.commit(retry, 0, 1, true));
    }
    #[test]
    fn cancelled_selection_callback_cannot_commit_a_new_attempt_in_the_same_life() {
        let mut i = Inventory::default();
        assert!(i.grant(0, 6, 0));
        let cancelled = i.reserve(0, 6, 0).unwrap();
        assert_eq!(i.reserve(0, 6, 0), Some(cancelled));
        assert!(i.cancel(cancelled));
        let current = i.reserve(0, 6, 0).unwrap();
        assert_ne!(cancelled, current);
        assert!(!i.commit(cancelled, 0, 0, true));
        assert!(!i.cancel(cancelled));
        assert!(i.contains(6));
        assert!(i.commit(current, 0, 0, true));
    }
}
