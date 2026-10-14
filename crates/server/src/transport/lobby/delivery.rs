//! Bounded plaintext mailboxes for authenticated lobby connections.
//! The target connection's sole writer owns encryption and S2C sequence numbers.
#[cfg(test)]
use crate::backend::identity::session::Session;
use crate::backend::rooms::Rooms;
use crate::contracts::lobby::DirectedPush;
use crate::contracts::lobby::Identity;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{mpsc, watch};

struct Endpoint {
    generation: u64,
    sender: mpsc::Sender<Vec<u8>>,
    closed: watch::Sender<bool>,
}
#[derive(Default)]
pub(crate) struct DeliveryHub {
    generation: AtomicU64,
    connections: Mutex<HashMap<Identity, Endpoint>>,
}
pub(crate) struct Subscription {
    identity: Identity,
    generation: u64,
    hub: Arc<DeliveryHub>,
    rooms: Arc<tokio::sync::Mutex<Rooms>>,
    receiver: mpsc::Receiver<Vec<u8>>,
    closed: watch::Receiver<bool>,
}
impl DeliveryHub {
    pub fn subscribe(
        self: &Arc<Self>,
        identity: Identity,
        rooms: Arc<tokio::sync::Mutex<Rooms>>,
    ) -> (Subscription, Vec<Identity>) {
        let (sender, receiver) = mpsc::channel(64);
        let (closed_sender, closed) = watch::channel(false);
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut entries = self
            .connections
            .lock()
            .expect("lobby delivery lock poisoned");
        let old: Vec<_> = entries
            .keys()
            .copied()
            .filter(|key| key.title == identity.title && key.user == identity.user)
            .collect();
        for key in &old {
            if let Some(endpoint) = entries.remove(key) {
                endpoint.closed.send_replace(true);
            }
        }
        entries.insert(
            identity,
            Endpoint {
                generation,
                sender,
                closed: closed_sender,
            },
        );
        (
            Subscription {
                identity,
                generation,
                hub: self.clone(),
                rooms,
                receiver,
                closed,
            },
            old.into_iter().filter(|key| *key != identity).collect(),
        )
    }
    fn detach(&self, identity: Identity, generation: u64) -> bool {
        let mut entries = self
            .connections
            .lock()
            .expect("lobby delivery lock poisoned");
        if entries
            .get(&identity)
            .is_some_and(|entry| entry.generation == generation)
        {
            entries.remove(&identity);
            true
        } else {
            false
        }
    }
    pub fn publish(&self, deliveries: &[DirectedPush]) -> Vec<Identity> {
        let mut entries = self
            .connections
            .lock()
            .expect("lobby delivery lock poisoned");
        let mut failed = Vec::new();
        for push in deliveries {
            if let Some(endpoint) = entries.get(&push.target)
                && endpoint.sender.try_send(push.body.clone()).is_err()
            {
                if let Some(endpoint) = entries.remove(&push.target) {
                    endpoint.closed.send_replace(true);
                }
                failed.push(push.target);
            }
        }
        failed
    }
}
impl Subscription {
    pub fn is_current(&self) -> bool {
        self.hub
            .connections
            .lock()
            .expect("lobby delivery lock poisoned")
            .get(&self.identity)
            .is_some_and(|entry| entry.generation == self.generation)
    }
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        if *self.closed.borrow() {
            return None;
        }
        tokio::select! {
            _ = self.closed.changed() => None,
            message = self.receiver.recv() => message,
        }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if !self.hub.detach(self.identity, self.generation) {
            return;
        }
        let rooms = self.rooms.clone();
        let hub = self.hub.clone();
        let identity = self.identity;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let pushes = rooms.lock().await.disconnect(identity);
                publish_with_cleanup(&hub, &rooms, pushes).await;
            });
        }
    }
}
pub(crate) async fn publish_with_cleanup(
    hub: &DeliveryHub,
    rooms: &tokio::sync::Mutex<Rooms>,
    mut pushes: Vec<DirectedPush>,
) {
    while !pushes.is_empty() {
        let failed = hub.publish(&pushes);
        pushes.clear();
        let mut directory = rooms.lock().await;
        for identity in failed {
            pushes.extend(directory.disconnect(identity));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            title_id: 1002,
            user_id: 42,
            iv_seed: 1,
            expires_at: u32::MAX,
            key: [0; 24],
        }
    }
    #[tokio::test]
    async fn mailbox_routes_only_to_the_exact_identity_and_replacement_preserves_new_lease() {
        let rooms = Arc::new(tokio::sync::Mutex::new(Rooms::default()));
        let hub = Arc::new(DeliveryHub::default());
        let actor = session();
        let (mut old, _) = hub.subscribe((&actor).into(), rooms.clone());
        let (mut current, replaced) = hub.subscribe((&actor).into(), rooms.clone());
        assert!(replaced.is_empty());
        assert!(!old.is_current());
        assert!(old.next().await.is_none());
        drop(old);
        assert!(current.is_current());
        let mut other = actor.clone();
        other.iv_seed += 1;
        hub.publish(&[DirectedPush {
            target: (&other).into(),
            body: vec![99],
        }]);
        assert!(current.receiver.try_recv().is_err());
        hub.publish(&[DirectedPush {
            target: (&actor).into(),
            body: vec![1, 2, 3],
        }]);
        assert_eq!(current.next().await, Some(vec![1, 2, 3]));
    }
    #[tokio::test]
    async fn slow_recipient_is_closed_instead_of_silently_losing_room_events() {
        let rooms = Arc::new(tokio::sync::Mutex::new(Rooms::default()));
        let hub = Arc::new(DeliveryHub::default());
        let actor = session();
        let identity = Identity::from(&actor);
        let (mut receiver, _) = hub.subscribe(identity, rooms);
        for _ in 0..64 {
            assert!(
                hub.publish(&[DirectedPush {
                    target: identity,
                    body: vec![1]
                }])
                .is_empty()
            );
        }
        assert_eq!(
            hub.publish(&[DirectedPush {
                target: identity,
                body: vec![2]
            }]),
            vec![identity]
        );
        assert!(!receiver.is_current());
        assert!(receiver.next().await.is_none());
    }
}
