//! Per-session local room channels. Game sessions and gameplay are separate.
use crate::{services::TaskResult, session::Session};
use anyhow::Result;
use codol_protocol::room::Description;
use codol_protocol::task::{self, Reader, Request, Writer};
use std::collections::{HashMap, VecDeque};

struct Channel {
    title_id: u32,
    owner: u64,
    session_seed: u32,
    expires_at: u32,
    game: Option<GameSession>,
    messages: VecDeque<Vec<u8>>,
}
struct GameSession {
    id: [u8; 8],
    description: Description,
}
#[derive(Default)]
pub(crate) struct Rooms {
    channels: HashMap<u64, Channel>,
}
impl Rooms {
    pub fn handle(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        self.channels.retain(|_, c| c.expires_at > now);
        match (request.service, request.task) {
            (11, 13) => {
                Reader::new(request.arguments).finish()?;
                self.channels.retain(|_, c| {
                    !(c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed != session.iv_seed)
                });
                let existing = self
                    .channels
                    .iter()
                    .find(|(_, c)| {
                        c.title_id == session.title_id
                            && c.owner == session.user_id
                            && c.session_seed == session.iv_seed
                    })
                    .map(|(id, _)| *id);
                let id = if let Some(id) = existing {
                    id
                } else {
                    if self.channels.len() >= 128 {
                        return Ok(TaskResult::failure(
                            "room_capacity_reached",
                            task::TASK_FAILED,
                        ));
                    }
                    let mut nonce = [0u8; 8];
                    let id = loop {
                        openssl::rand::rand_bytes(&mut nonce)?;
                        let id = u64::from_le_bytes(nonce);
                        if id != 0 && id != u64::MAX && !self.channels.contains_key(&id) {
                            break id;
                        }
                    };
                    self.channels.insert(
                        id,
                        Channel {
                            title_id: session.title_id,
                            owner: session.user_id,
                            session_seed: session.iv_seed,
                            expires_at: session.expires_at,
                            game: None,
                            messages: VecDeque::new(),
                        },
                    );
                    id
                };
                // Target bdChannelInfo decoder 0xa55a40, bound by 0xa4c940.
                let mut record = Writer::default();
                record.u64(id);
                record.u32(1);
                record.u32(0);
                record.u64(session.user_id);
                record.boolean(false);
                record.boolean(false);
                record.u32(0);
                Ok(TaskResult::success(
                    "local_room_channel_created",
                    vec![record.0],
                ))
            }
            (11, 2) => {
                let mut r = Reader::new(request.arguments);
                let id = r.u64()?;
                r.finish()?;
                if !self.channels.get(&id).is_some_and(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                }) {
                    return Ok(TaskResult::failure(
                        "room_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                }
                self.channels.remove(&id);
                Ok(TaskResult::success("local_room_channel_left", vec![]))
            }
            (21, 1) => {
                let description = Description::parse(request.arguments, session.user_id)?;
                let Some(channel) = self.channels.get_mut(&description.channel_id).filter(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                }) else {
                    return Ok(TaskResult::failure(
                        "room_channel_mismatch",
                        task::TASK_FAILED,
                    ));
                };
                let id = if let Some(game) = &channel.game {
                    game.id
                } else {
                    let mut id = [0u8; 8];
                    openssl::rand::rand_bytes(&mut id)?;
                    id
                };
                channel.game = Some(GameSession { id, description });
                let mut record = Writer::default();
                record.blob(&id)?;
                Ok(TaskResult::success(
                    "local_room_session_created",
                    vec![record.0],
                ))
            }
            (21, 12) => {
                let mut r = Reader::new(request.arguments);
                let id = r.blob()?;
                if id.len() != 8 {
                    return Ok(TaskResult::failure(
                        "invalid_room_session_id",
                        task::TASK_FAILED,
                    ));
                }
                let _update_kind = r.u32()?;
                let description = Description::parse(r.remaining(), session.user_id)?;
                let Some(channel) = self.channels.get_mut(&description.channel_id).filter(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                }) else {
                    return Ok(TaskResult::failure(
                        "room_channel_mismatch",
                        task::TASK_FAILED,
                    ));
                };
                let Some(game) = channel.game.as_mut().filter(|g| g.id == id) else {
                    return Ok(TaskResult::failure(
                        "room_session_mismatch",
                        task::TASK_FAILED,
                    ));
                };
                game.description = description;
                Ok(TaskResult::success("local_room_session_updated", vec![]))
            }
            (21, 3) => {
                let mut r = Reader::new(request.arguments);
                let id = r.blob()?;
                r.finish()?;
                let Some(channel) = self.channels.values_mut().find(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                        && c.game.as_ref().is_some_and(|g| g.id == id)
                }) else {
                    return Ok(TaskResult::failure(
                        "room_session_mismatch",
                        task::TASK_FAILED,
                    ));
                };
                channel.game = None;
                Ok(TaskResult::success("local_room_session_deleted", vec![]))
            }
            (11, 15) => {
                let mut r = Reader::new(request.arguments);
                let id = r.u64()?;
                let maximum = r.u32()?;
                r.finish()?;
                let Some(channel) = self.channels.get(&id).filter(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                        && (1..=100).contains(&maximum)
                }) else {
                    return Ok(TaskResult::failure(
                        "room_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                };
                let Some(game) = &channel.game else {
                    return Ok(TaskResult::failure(
                        "room_session_missing",
                        task::TASK_FAILED,
                    ));
                };
                // Target 0xa55c20: UInt64, String[64], UInt32.
                let mut record = Writer::default();
                record.u64(channel.owner);
                record.string(&game.description.owner_name);
                record.u32(0);
                Ok(TaskResult::success("local_room_members", vec![record.0]))
            }
            (11, 4) => {
                let mut r = Reader::new(request.arguments);
                let id = r.u64()?;
                let batch = r.blob()?;
                let delivery = r.u32()?;
                r.finish()?;
                let Some(channel) = self.channels.get_mut(&id).filter(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                        && c.game.is_some()
                }) else {
                    return Ok(TaskResult::failure(
                        "room_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                };
                if batch.is_empty() || batch.len() > 16384 || delivery > 1 {
                    return Ok(TaskResult::failure(
                        "unsupported_room_message_batch",
                        task::TASK_FAILED,
                    ));
                }
                let mut left = batch;
                let mut count = 0;
                while !left.is_empty() {
                    let header = left
                        .get(..4)
                        .ok_or_else(|| anyhow::anyhow!("truncated room message length"))?;
                    let n = u32::from_le_bytes(header.try_into()?) as usize;
                    anyhow::ensure!(
                        n > 0 && n <= left.len() - 4 && count < 64,
                        "invalid room message framing"
                    );
                    left = &left[4 + n..];
                    count += 1;
                }
                // Single-owner room: accept the ordered batch. No other member
                // has joined yet, so there are no remote recipients to forward to.
                // Keep a bounded queue for subsequent room-message recovery.
                if channel.messages.len() == 32 {
                    channel.messages.pop_front();
                }
                channel.messages.push_back(batch.to_vec());
                Ok(TaskResult::success("local_room_message_accepted", vec![]))
            }
            _ => Ok(TaskResult::failure(
                "unsupported_room_task",
                task::TASK_FAILED,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(uid: u64) -> Session {
        Session {
            title_id: 1002,
            user_id: uid,
            iv_seed: 1,
            expires_at: 100,
            key: [0; 24],
        }
    }
    fn create(rooms: &mut Rooms, s: &Session) -> (u64, Vec<u8>) {
        let reply = rooms
            .handle(&Request::parse(&[11, 3, 13, 0]).unwrap(), s, 1)
            .unwrap()
            .result
            .unwrap();
        let id = Reader::new(&reply[0]).u64().unwrap();
        (id, reply[0].clone())
    }
    #[test]
    fn game_sessions_require_owned_channels_and_updates_require_the_issued_id() {
        let mut rooms = Rooms::default();
        let owner = session(0x8000000000000001);
        let (channel_id, _) = create(&mut rooms, &owner);
        let mut description =
            include_bytes!("../../../fixtures/protocol/room-create-synthetic.bin").to_vec();
        let n = description.len();
        description[n - 19..n - 11].copy_from_slice(&channel_id.to_le_bytes());
        let mut body = vec![21, 3, 1];
        body.extend_from_slice(&description);
        let request = Request::parse(&body).unwrap();
        let result = rooms.handle(&request, &owner, 1).unwrap().result.unwrap();
        let id = Reader::new(&result[0]).blob().unwrap().to_vec();
        assert_eq!(id.len(), 8);
        assert_eq!(
            rooms.handle(&request, &owner, 1).unwrap().result.unwrap(),
            result
        );
        assert!(rooms.handle(&request, &session(2), 1).is_err());
        let mut other_title = owner.clone();
        other_title.title_id += 1;
        assert!(
            rooms
                .handle(&request, &other_title, 1)
                .unwrap()
                .result
                .is_err()
        );
        let mut update = Writer(vec![21, 3, 12]);
        update.blob(&id).unwrap();
        update.u32(1);
        update.0.extend_from_slice(&description);
        assert!(
            rooms
                .handle(&Request::parse(&update.0).unwrap(), &owner, 1)
                .unwrap()
                .result
                .is_ok()
        );
        update.0[9] ^= 1;
        assert!(
            rooms
                .handle(&Request::parse(&update.0).unwrap(), &owner, 1)
                .unwrap()
                .result
                .is_err()
        );
        let mut members = Writer(vec![11, 3, 15]);
        members.u64(channel_id);
        members.u32(100);
        members.0.push(0);
        let reply = rooms
            .handle(&Request::parse(&members.0).unwrap(), &owner, 1)
            .unwrap()
            .result
            .unwrap();
        let mut reader = Reader::new(&reply[0]);
        assert_eq!(reader.u64().unwrap(), owner.user_id);
        assert_eq!(reader.string(65).unwrap(), "Lab player");
        assert_eq!(reader.u32().unwrap(), 0);
        let batch = [12, 0, 0, 0, 2, 2, 0, 0, 0, 8, 0, 0, 0, 128, 8, 0];
        let mut message = Writer(vec![11, 3, 4]);
        message.u64(channel_id);
        message.blob(&batch).unwrap();
        message.u32(1);
        message.0.push(0);
        let req = Request::parse(&message.0).unwrap();
        assert!(rooms.handle(&req, &session(2), 1).unwrap().result.is_err());
        for _ in 0..40 {
            assert!(rooms.handle(&req, &owner, 1).unwrap().result.is_ok());
        }
        assert_eq!(rooms.channels[&channel_id].messages.len(), 32);
        assert_eq!(rooms.channels[&channel_id].messages.back().unwrap(), &batch);
        message.0[18] = 255; // First framed length now exceeds the blob.
        assert!(
            rooms
                .handle(&Request::parse(&message.0).unwrap(), &owner, 1)
                .is_err()
        );
        let mut delete = Writer(vec![21, 3, 3]);
        delete.blob(&id).unwrap();
        delete.0.push(0);
        let req = Request::parse(&delete.0).unwrap();
        assert!(rooms.handle(&req, &session(2), 1).unwrap().result.is_err());
        assert!(rooms.handle(&req, &owner, 1).unwrap().result.is_ok());
        assert!(rooms.channels[&channel_id].game.is_none());
    }
    #[test]
    fn channels_are_idempotent_isolated_and_expire_with_the_session() {
        let mut rooms = Rooms::default();
        let a = session(u64::MAX);
        let b = session(2);
        let (aid, record) = create(&mut rooms, &a);
        assert_eq!(create(&mut rooms, &a).0, aid);
        assert_ne!(create(&mut rooms, &b).0, aid);
        assert_eq!(
            &record[9..],
            &[
                8, 1, 0, 0, 0, 8, 0, 0, 0, 0, 10, 255, 255, 255, 255, 255, 255, 255, 255, 1, 0, 1,
                0, 8, 0, 0, 0, 0
            ]
        );
        let mut w = Writer(vec![11, 3, 2]);
        w.u64(aid);
        w.0.push(0);
        let leave = Request::parse(&w.0).unwrap();
        let mut other_title = a.clone();
        other_title.title_id += 1;
        for other in [&b, &other_title] {
            assert!(rooms.handle(&leave, other, 1).unwrap().result.is_err());
        }
        assert!(rooms.handle(&leave, &a, 1).unwrap().result.is_ok());
        assert!(!rooms.channels.contains_key(&aid));
        let new_id = create(&mut rooms, &a).0;
        let mut new_session = a.clone();
        new_session.iv_seed += 1;
        assert_ne!(create(&mut rooms, &new_session).0, new_id);
        assert!(!rooms.channels.contains_key(&new_id));
        let bad = Request::parse(&[11, 3, 13, 0, 0]).unwrap();
        assert!(rooms.handle(&bad, &a, 101).is_err());
        assert!(rooms.channels.is_empty());
    }
}
