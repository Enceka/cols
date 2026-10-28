//! Lobby public chat groups: native subscription state and text fan-out.
//! The frontend keys public text by UI channel 0, and the native send branch
//! uses a fixed group id, so membership decides which connections get events.
//! The native subscription builder subscribes the newbie channel with its own
//! id, so both ids name one channel and delivery compares them as one.
use crate::backend::identity::session::Session;
use crate::contracts::lobby::Identity;
use crate::contracts::lobby::TaskResult;
use anyhow::{Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::task;
use codol_protocol::lobby::task::Reader;
use codol_protocol::lobby::task::Request;
use codol_protocol::lobby::task::Writer;
use std::collections::HashMap;

/// Tagged group type written by native builders 0xa4d500 and 0xa4d810.
pub const GROUP_TYPE: u8 = 130;
/// Largest group id set accepted in one subscription change.
const MAX_GROUPS: u32 = 16;
/// `ChattingController.encodeMsg` marks public text for UI channel 0.
const PUBLIC_MARKER: &[u8] = b"tUtoR0";
/// Native event id for a public group message.
const PUBLIC_EVENT: u32 = 34;
/// Longest public text accepted after the frontend marker.
const MAX_TEXT: usize = 1024;
/// Group id of the native send branch for public text.
const PUBLIC_SEND_GROUP: u64 = 10;
/// Group id the native subscription builder passes as `ServerChannel.NEWBIE`.
const NEWBIE_GROUP: u64 = 99;

#[derive(Default)]
pub(crate) struct Chat {
    /// Connections holding a lobby subscription, keyed by authenticated identity.
    members: HashMap<Identity, Member>,
    /// Subscribed group ids per authenticated identity.
    groups: HashMap<Identity, Vec<u64>>,
}

struct Member {
    name: String,
}

impl Chat {
    /// Service 102 task 1 changes subscriptions and task 3 sends public text.
    pub(crate) fn handle(
        &mut self,
        request: &Request<'_>,
        session: &Session,
    ) -> Result<TaskResult> {
        let identity = Identity::from(session);
        ensure!(
            self.members.len() < 4096 || self.members.contains_key(&identity),
            "chat member limit"
        );
        self.members.entry(identity).or_insert_with(|| Member {
            name: STANDARD.encode(identity.user.to_string()),
        });
        match request.task {
            1 => self.groups(request, identity),
            3 => self.send(request, identity),
            _ => Ok(TaskResult::failure(
                "unsupported_chat_task",
                task::TASK_FAILED,
            )),
        }
    }

    /// The authenticated lobby nickname replaces the placeholder sender name.
    pub(crate) fn set_name(&mut self, session: &Session, name: String) {
        self.members
            .entry(Identity::from(session))
            .or_insert_with(|| Member {
                name: STANDARD.encode(session.user_id.to_string()),
            })
            .name = STANDARD.encode(name.as_bytes());
    }

    /// A closed lobby connection loses its nickname and subscriptions.
    pub(crate) fn disconnect(&mut self, identity: Identity) {
        self.members.remove(&identity);
        self.groups.remove(&identity);
    }
    fn groups(&mut self, request: &Request<'_>, identity: Identity) -> Result<TaskResult> {
        let mut r = Reader::new(request.arguments);
        let removed = tagged_groups(&mut r)?;
        let added = tagged_groups(&mut r)?;
        r.finish()?;
        ensure!(
            removed
                .iter()
                .chain(&added)
                .all(|&(kind, _)| kind == GROUP_TYPE),
            "unsupported public chat group type"
        );
        let current = self.groups.get(&identity).cloned().unwrap_or_default();
        let mut next: Vec<u64> = Vec::new();
        // Keep the native insertion order and drop repeats.
        for id in current
            .iter()
            .copied()
            .filter(|id| !removed.iter().any(|&(_, gone)| gone == *id))
            .chain(added.iter().map(|&(_, id)| id))
        {
            if !next.contains(&id) && next.len() < MAX_GROUPS as usize {
                next.push(id);
            }
        }
        if next.is_empty() {
            self.groups.remove(&identity);
        } else {
            self.groups.insert(identity, next.clone());
        }
        let mut record = Writer::default();
        record.u32(next.len() as u32);
        for id in next {
            record.u8(GROUP_TYPE);
            record.u64(id);
        }
        Ok(TaskResult::success("public_chat_groups", vec![record.0]))
    }

    fn send(&mut self, request: &Request<'_>, identity: Identity) -> Result<TaskResult> {
        let mut r = Reader::new(request.arguments);
        let kind = r.u8()?;
        let group = r.u64()?;
        let payload = r.blob()?;
        r.finish()?;
        ensure!(kind == GROUP_TYPE, "unsupported public chat group type");
        public_text(payload)?;
        let group = public_channel(group);
        let name = self.members[&identity].name.clone();
        // Native event 34 carries the send payload tagged as one blob.
        let mut body = Writer::default();
        body.u32(PUBLIC_EVENT);
        body.u64(identity.user);
        body.string(&name);
        body.blob(payload)?;
        let mut targets: Vec<Identity> = self
            .groups
            .iter()
            .filter(|(_, groups)| groups.iter().any(|&id| public_channel(id) == group))
            .filter(|(member, _)| self.members.contains_key(member))
            .map(|(member, _)| *member)
            .collect();
        targets.sort_by_key(|target| target.user);
        let mut result = TaskResult::success("public_chat_message", vec![]);
        for target in targets {
            result.push_to(identity, target, body.0.clone());
        }
        Ok(result)
    }
}

/// The newbie subscription and the public send branch name one channel.
fn public_channel(id: u64) -> u64 {
    if id == NEWBIE_GROUP {
        PUBLIC_SEND_GROUP
    } else {
        id
    }
}

/// LE32(prefix length 0) then LE32(size) and the frontend-marked text.
fn public_text(payload: &[u8]) -> Result<&str> {
    let prefix = payload
        .get(..4)
        .ok_or_else(|| anyhow!("public chat header"))?;
    ensure!(
        u32::from_le_bytes(prefix.try_into()?) == 0,
        "unsupported public chat prefix"
    );
    let size = u32::from_le_bytes(
        payload
            .get(4..8)
            .ok_or_else(|| anyhow!("public chat length"))?
            .try_into()?,
    ) as usize;
    ensure!(
        size <= MAX_TEXT + PUBLIC_MARKER.len(),
        "public chat text length"
    );
    let text = payload
        .get(8..8 + size)
        .ok_or_else(|| anyhow!("public chat text truncated"))?;
    ensure!(payload.len() == 8 + size, "public chat trailing data");
    let text = text.strip_prefix(PUBLIC_MARKER).unwrap_or(text);
    ensure!(!text.is_empty(), "public chat text is empty");
    std::str::from_utf8(text).map_err(|_| anyhow!("public chat text encoding"))
}

fn tagged_groups(r: &mut Reader<'_>) -> Result<Vec<(u8, u64)>> {
    let count = r.u32()?;
    ensure!(count <= 16u32, "chat group count");
    (0..count).map(|_| Ok((r.u8()?, r.u64()?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(user: u64, seed: u32) -> Session {
        Session {
            title_id: 1002,
            user_id: user,
            iv_seed: seed,
            expires_at: u32::MAX,
            key: [seed as u8; 24],
        }
    }

    fn groups(added: &[(u8, u64)], removed: &[(u8, u64)]) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.u32(removed.len() as u32);
        for &(kind, id) in removed {
            writer.u8(kind);
            writer.u64(id);
        }
        writer.u32(added.len() as u32);
        for &(kind, id) in added {
            writer.u8(kind);
            writer.u64(id);
        }
        writer.0.push(0);
        writer.0
    }

    fn send(group: u64, text: &[u8]) -> Vec<u8> {
        let mut payload = vec![0; 4];
        payload.extend_from_slice(&(text.len() as u32).to_le_bytes());
        payload.extend_from_slice(text);
        let mut writer = Writer::default();
        writer.u8(GROUP_TYPE);
        writer.u64(group);
        writer.blob(&payload).unwrap();
        writer.0.push(0);
        writer.0
    }

    #[test]
    fn public_chat_fans_out_only_to_subscribed_exact_identities() {
        let mut chat = Chat::default();
        let sender = session(1, 10);
        let recipient = session(2, 20);
        let stale = session(2, 21);
        chat.set_name(&sender, "Sender".to_owned());
        let subscribe = groups(&[(GROUP_TYPE, 10)], &[]);
        assert!(
            chat.handle(
                &Request {
                    service: 102,
                    task: 1,
                    arguments: &subscribe,
                },
                &sender,
            )
            .unwrap()
            .result
            .is_ok()
        );
        assert!(
            chat.handle(
                &Request {
                    service: 102,
                    task: 1,
                    arguments: &subscribe,
                },
                &recipient,
            )
            .unwrap()
            .result
            .is_ok()
        );
        let message = send(10, b"tUtoR0hello");
        let result = chat
            .handle(
                &Request {
                    service: 102,
                    task: 3,
                    arguments: &message,
                },
                &sender,
            )
            .unwrap();
        assert_eq!(result.pushes.len(), 1);
        assert_eq!(result.deliveries.len(), 1);
        assert_eq!(result.deliveries[0].target, Identity::from(&recipient));
        let mut event = Reader::new(&result.pushes[0]);
        assert_eq!(event.u32().unwrap(), PUBLIC_EVENT);
        assert_eq!(event.u64().unwrap(), sender.user_id);
        assert_eq!(event.string(64).unwrap(), STANDARD.encode(b"Sender"));
        assert_eq!(
            chat.groups.get(&Identity::from(&recipient)),
            Some(&vec![10])
        );
        assert!(!chat.groups.contains_key(&Identity::from(&stale)));
    }

    #[test]
    fn public_chat_rejects_trailing_payload_and_invalid_text() {
        let mut chat = Chat::default();
        let actor = session(1, 10);
        let subscribe = groups(&[(GROUP_TYPE, 10)], &[]);
        chat.handle(
            &Request {
                service: 102,
                task: 1,
                arguments: &subscribe,
            },
            &actor,
        )
        .unwrap();
        let mut trailing = send(10, b"tUtoR0hello");
        trailing.push(1);
        assert!(
            chat.handle(
                &Request {
                    service: 102,
                    task: 3,
                    arguments: &trailing,
                },
                &actor,
            )
            .is_err()
        );
        assert!(
            chat.handle(
                &Request {
                    service: 102,
                    task: 3,
                    arguments: &send(10, b"tUtoR0\xff"),
                },
                &actor,
            )
            .is_err()
        );
    }

    #[test]
    fn newbie_subscription_receives_public_send() {
        let mut chat = Chat::default();
        let sender = session(1, 10);
        let newbie = session(2, 20);
        let other = session(3, 30);
        chat.set_name(&sender, "Sender".to_owned());
        let subscribe = groups(&[(GROUP_TYPE, NEWBIE_GROUP)], &[]);
        let request = Request {
            service: 102,
            task: 1,
            arguments: &subscribe,
        };
        assert!(chat.handle(&request, &newbie).unwrap().result.is_ok());
        let unrelated = groups(&[(GROUP_TYPE, 7)], &[]);
        let request = Request {
            service: 102,
            task: 1,
            arguments: &unrelated,
        };
        assert!(chat.handle(&request, &other).unwrap().result.is_ok());
        let message = send(PUBLIC_SEND_GROUP, b"tUtoR0hello");
        let request = Request {
            service: 102,
            task: 3,
            arguments: &message,
        };
        let result = chat.handle(&request, &sender).unwrap();
        assert_eq!(result.deliveries.len(), 1);
        assert_eq!(result.deliveries[0].target, Identity::from(&newbie));
        assert_eq!(
            chat.groups.get(&Identity::from(&newbie)),
            Some(&vec![NEWBIE_GROUP])
        );
    }
}
