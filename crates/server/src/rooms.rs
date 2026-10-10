//! Per-session local room channels. Game sessions and gameplay are separate.
use crate::{services::TaskResult, session::Session};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::room::Description;
use codol_protocol::task::{self, Reader, Request, Writer};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

struct Channel {
    title_id: u32,
    owner: u64,
    session_seed: u32,
    expires_at: u32,
    game: Option<GameSession>,
    messages: VecDeque<Vec<u8>>,
    bots_announced: bool,
    roster_requested: bool,
    member_teams: HashMap<u64, u8>,
}
struct GameSession {
    id: [u8; 8],
    description: Description,
}
fn bot_message(client: u8, channel_id: u64, owner: u64, record: &[u8]) -> Result<Vec<u8>> {
    let mut push = Writer::default();
    push.u32(7);
    push.u64(codol_protocol::room_messages::bot_id(client));
    push.string(&STANDARD.encode(codol_protocol::room_messages::bot_name(client)));
    push.u64(channel_id);
    push.blob(&codol_protocol::room_messages::to_member(record, owner)?)?;
    push.u32(1);
    Ok(push.0)
}
impl Channel {
    fn update_members(&mut self, updates: &codol_protocol::room_messages::Updates) -> Result<()> {
        let mut candidate = if updates.full_roster {
            HashMap::new()
        } else {
            self.member_teams.clone()
        };
        for member in &updates.members {
            anyhow::ensure!(
                member.id == self.owner
                    || (1..=11).any(|c| codol_protocol::room_messages::bot_id(c) == member.id),
                "unexpected member in local bot room"
            );
            anyhow::ensure!(member.team <= 3, "invalid room team");
            candidate.insert(member.id, member.team);
        }
        self.member_teams = candidate;
        Ok(())
    }
    fn game_teams(&self) -> Result<[u8; 12]> {
        let mut teams = [0; 12];
        for (client, team) in teams.iter_mut().enumerate() {
            let id = if client == 0 {
                self.owner
            } else {
                codol_protocol::room_messages::bot_id(client as u8)
            };
            let room_team = self.member_teams.get(&id).copied().unwrap_or(0);
            anyhow::ensure!(
                matches!(room_team, 1 | 2),
                "bot room roster not ready for team play"
            );
            *team = 3 - room_team;
        }
        anyhow::ensure!(
            teams.iter().filter(|&&t| t == 1).count() == 6,
            "bot room requires six players per team"
        );
        Ok(teams)
    }
}
fn bot_pushes(
    policy: &crate::room_bots::Policy,
    channel_id: u64,
    session: &Session,
) -> Result<Vec<Vec<u8>>> {
    let mut pushes = Vec::new();
    for client in 1..=codol_protocol::room_messages::BOT_CLIENTS {
        let uid = codol_protocol::room_messages::bot_id(client);
        anyhow::ensure!(uid != session.user_id, "bot identity overlaps room owner");
        // 0x6d3830 Base64-decodes SDK names before publishing room members.
        // Match snapshots use the plain display name through a separate codec.
        let name = STANDARD.encode(codol_protocol::room_messages::bot_name(client));
        // SDK channel membership must precede the room-level JoinRequest.
        // C1 0x6e8a20 silently drops even a valid join if its sender is absent
        // from the channel table populated by event 9 (0xa3d730 -> 0x6c9ba0).
        let mut joined = Writer::default();
        joined.u32(9);
        joined.u64(uid);
        joined.string(&name);
        joined.u64(channel_id);
        joined.u8(2); // Full member/channel metadata, preserving the owner.
        joined.u32(0); // Member subscription time.
        joined.u64(channel_id);
        joined.u32(u32::from(client) + 1);
        joined.u32(0);
        joined.u64(session.user_id);
        joined.boolean(false);
        joined.boolean(false);
        joined.u32(0);
        pushes.push(joined.0);
        for record in [
            codol_protocol::room_messages::bot_join(client, policy.version)?,
            codol_protocol::room_messages::bot_ready(),
        ] {
            let payload = codol_protocol::room_messages::to_member(&record, session.user_id)?;
            let mut push = Writer::default();
            push.u32(7);
            push.u64(uid);
            push.string(&name);
            push.u64(channel_id);
            push.blob(&payload)?;
            push.u32(1);
            pushes.push(push.0);
        }
    }
    Ok(pushes)
}
#[derive(Default)]
pub(crate) struct Rooms {
    channels: HashMap<u64, Channel>,
    game_endpoint: Option<Arc<crate::game_server::GameAdmissions>>,
}
impl Rooms {
    pub(crate) fn renew_session(&mut self, session: &Session, now: u32) -> Result<()> {
        if let Some(endpoint) = &self.game_endpoint {
            endpoint.renew_session(session, now)?;
        }
        for channel in self.channels.values_mut().filter(|c| {
            c.expires_at > now
                && c.title_id == session.title_id
                && c.owner == session.user_id
                && c.session_seed == session.iv_seed
        }) {
            channel.expires_at = session.expires_at;
        }
        Ok(())
    }
    pub fn with_game_endpoint(endpoint: Arc<crate::game_server::GameAdmissions>) -> Self {
        Self {
            channels: HashMap::new(),
            game_endpoint: Some(endpoint),
        }
    }
    pub fn handle(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        if session.expires_at > now {
            for channel in self.channels.values_mut().filter(|c| {
                c.title_id == session.title_id
                    && c.owner == session.user_id
                    && c.session_seed == session.iv_seed
            }) {
                channel.expires_at = session.expires_at;
            }
        }
        self.channels.retain(|_, c| c.expires_at > now);
        match (request.service, request.task) {
            (6, 14) if self.game_endpoint.is_some() => {
                let endpoint = self.game_endpoint.as_ref().expect("guarded endpoint");
                if let Some(channel) = self.channels.values().find(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                        && c.bots_announced
                }) {
                    endpoint.set_room_teams(session, channel.game_teams()?)?;
                }
                endpoint.control(request.arguments, session, now)
            }
            (21, 5) if self.game_endpoint.is_some() => {
                let query = codol_protocol::room::Search::parse(request.arguments)?;
                // This experimental endpoint allocates solo sessions. Peer-room
                // matching is intentionally not advertised as implemented.
                if query.query == 3 {
                    return Ok(TaskResult::success(
                        "local_solo_search_no_peer_rooms",
                        vec![],
                    ));
                }
                let Some(channel) = self.channels.values().find(|c| {
                    c.title_id == session.title_id
                        && c.owner == session.user_id
                        && c.session_seed == session.iv_seed
                }) else {
                    return Ok(TaskResult::failure(
                        "game_allocation_requires_channel",
                        task::TASK_FAILED,
                    ));
                };
                if channel
                    .game
                    .as_ref()
                    .is_some_and(|g| g.description.build != query.criteria[0])
                {
                    return Ok(TaskResult::failure(
                        "game_build_mismatch",
                        task::TASK_FAILED,
                    ));
                }
                let endpoint = self.game_endpoint.as_ref().expect("guarded game endpoint");
                let Some(game) = channel.game.as_ref() else {
                    return Ok(TaskResult::failure(
                        "game_allocation_requires_live_key",
                        task::TASK_FAILED,
                    ));
                };
                let (id, key) = endpoint.allocate(
                    session,
                    game.description.security_key,
                    game.description.build,
                    now,
                )?;
                let record = game
                    .description
                    .allocation_record(endpoint.address(), &id, &key)?;
                Ok(TaskResult::success(
                    "local_game_transport_allocated",
                    vec![record],
                ))
            }
            (11, 13) => {
                Reader::new(request.arguments).finish()?;
                if let Some(endpoint) = &self.game_endpoint {
                    endpoint.revoke_other_sessions(session);
                }
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
                            bots_announced: false,
                            roster_requested: false,
                            member_teams: HashMap::new(),
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
                if let Some(endpoint) = &self.game_endpoint {
                    endpoint.revoke(session);
                }
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
                // Wait for the owner's RoomInfo broadcast: creation's RPC
                // callback has not yet installed the room channel receiver.
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
                if let Some(endpoint) = &self.game_endpoint {
                    endpoint.revoke(session);
                }
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
                let mut records = vec![record.0];
                if channel.bots_announced {
                    for client in 1..=codol_protocol::room_messages::BOT_CLIENTS {
                        if records.len() >= maximum as usize {
                            break;
                        }
                        let mut bot = Writer::default();
                        bot.u64(codol_protocol::room_messages::bot_id(client));
                        bot.string(
                            &STANDARD.encode(codol_protocol::room_messages::bot_name(client)),
                        );
                        bot.u32(0);
                        records.push(bot.0);
                    }
                }
                Ok(TaskResult::success("local_room_members", records))
            }
            (11, 5) => {
                let mut r = Reader::new(request.arguments);
                let id = r.u64()?;
                let recipient = r.u64()?;
                let batch = r.blob()?;
                let delivery = r.u32()?;
                r.finish()?;
                let channel = self
                    .channels
                    .get_mut(&id)
                    .filter(|c| {
                        c.title_id == session.title_id
                            && c.owner == session.user_id
                            && c.session_seed == session.iv_seed
                            && c.bots_announced
                    })
                    .ok_or_else(|| anyhow::anyhow!("bot delivery requires owned channel"))?;
                let client = (1..=11)
                    .find(|&c| codol_protocol::room_messages::bot_id(c) == recipient)
                    .ok_or_else(|| anyhow::anyhow!("recipient is not a local room bot"))?;
                anyhow::ensure!(delivery <= 1 && !batch.is_empty(), "invalid bot delivery");
                let updates = codol_protocol::room_messages::updates(batch, recipient)?;
                channel.update_members(&updates)?;
                let mut result = TaskResult::success("local_room_bot_delivery", vec![]);
                if updates.join_result == Some(0) {
                    result.pushes.push(bot_message(
                        client,
                        id,
                        session.user_id,
                        &codol_protocol::room_messages::bot_ready(),
                    )?);
                    if client == 11 && !channel.roster_requested {
                        result.pushes.push(bot_message(
                            client,
                            id,
                            session.user_id,
                            &codol_protocol::room_messages::roster_request(),
                        )?);
                        channel.roster_requested = true;
                    }
                }
                if updates.full_roster {
                    tracing::info!(teams=?channel.game_teams()?, "native bot room roster confirmed");
                }
                Ok(result)
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
                let updates = if channel.bots_announced
                    || self
                        .game_endpoint
                        .as_ref()
                        .is_some_and(|e| e.bot_rooms.is_some())
                {
                    codol_protocol::room_messages::bot_room_updates(batch, session.user_id)?
                } else {
                    Default::default()
                };
                if channel.bots_announced {
                    channel.update_members(&updates)?;
                    if updates.full_roster {
                        tracing::info!(teams=?channel.game_teams()?,"native bot room roster confirmed");
                    }
                }
                let mut left = batch;
                let mut count = 0;
                let mut chunks = Vec::<Vec<u8>>::new();
                let mut chunk = Vec::new();
                while !left.is_empty() {
                    let header = left
                        .get(..4)
                        .ok_or_else(|| anyhow::anyhow!("truncated room message length"))?;
                    let n = u32::from_le_bytes(header.try_into()?) as usize;
                    anyhow::ensure!(
                        n > 0 && n <= left.len() - 4 && n + 4 <= 1024 && count < 64,
                        "invalid room message framing"
                    );
                    if chunk.len() + 4 + n > 1024 {
                        chunks.push(std::mem::take(&mut chunk));
                    }
                    chunk.extend_from_slice(&left[..4 + n]);
                    left = &left[4 + n..];
                    count += 1;
                }
                if !chunk.is_empty() {
                    chunks.push(chunk);
                }
                // The owner also consumes its channel broadcast to update ready
                // state. Target event 7 (0xa42db0→0xa3d5a0) allows a 1024-byte blob.
                if channel.messages.len() == 32 {
                    channel.messages.pop_front();
                }
                channel.messages.push_back(batch.to_vec());
                let fallback = session.user_id.to_string();
                let name = channel
                    .game
                    .as_ref()
                    .map_or(fallback.as_str(), |g| g.description.owner_name.as_str());
                let mut result = TaskResult::success("local_room_message_broadcast", vec![]);
                for payload in chunks {
                    let mut push = Writer::default();
                    push.u32(7);
                    push.u64(session.user_id);
                    push.string(name);
                    push.u64(id);
                    push.blob(&payload)?;
                    push.u32(delivery);
                    result.pushes.push(push.0);
                }
                // Map changes and returning from a match reset native ready
                // flags. The human chooses when to start the next round;
                // simulated peers then rejoin readiness for that room state.
                if channel.bots_announced && updates.ready == Some(true) {
                    for client in 1..=codol_protocol::room_messages::BOT_CLIENTS {
                        result.pushes.push(bot_message(
                            client,
                            id,
                            session.user_id,
                            &codol_protocol::room_messages::bot_ready(),
                        )?);
                    }
                }
                if !channel.bots_announced
                    && let Some(policy) = self
                        .game_endpoint
                        .as_ref()
                        .and_then(|e| e.bot_rooms.as_ref())
                    && channel
                        .game
                        .as_ref()
                        .is_some_and(|g| g.description.build == -688906341)
                    && let Some(info) = updates.info
                    && policy.accepts(&info)
                {
                    result.pushes.extend(bot_pushes(policy, id, session)?);
                    channel.bots_announced = true;
                    tracing::info!(
                        map = info.map,
                        mode = info.mode,
                        bots = 11,
                        "native room bot participation queued"
                    );
                }
                Ok(result)
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
    #[test]
    fn channel_membership_precedes_each_bot_join_and_ready() {
        let owner = session(42);
        let policy = crate::room_bots::Policy {
            version: 7,
            maps: Default::default(),
        };
        let pushes = bot_pushes(&policy, 123, &owner).unwrap();
        assert_eq!(pushes.len(), 33);
        for (index, events) in pushes.chunks_exact(3).enumerate() {
            let client = index as u8 + 1;
            let uid = codol_protocol::room_messages::bot_id(client);
            let name = STANDARD.encode(codol_protocol::room_messages::bot_name(client));
            if client == 1 {
                assert_eq!(name, "Qm90IDE=");
            }
            if client == 11 {
                assert_eq!(name, "Qm90IDEx");
            }
            // Independent tagged SDK decoder contract, 0xa3d730/0xa55a40.
            let mut r = Reader::new(&events[0]);
            assert_eq!(r.u32().unwrap(), 9);
            assert_eq!(r.u64().unwrap(), uid);
            assert_eq!(r.string(65).unwrap(), name);
            assert_eq!(r.u64().unwrap(), 123);
            assert_eq!(r.u8().unwrap(), 2);
            assert_eq!(r.u32().unwrap(), 0);
            assert_eq!(r.u64().unwrap(), 123);
            assert_eq!(r.u32().unwrap(), u32::from(client) + 1);
            assert_eq!(r.u32().unwrap(), 0);
            assert_eq!(r.u64().unwrap(), owner.user_id);
            assert!(!r.boolean().unwrap());
            assert!(!r.boolean().unwrap());
            assert_eq!(r.u32().unwrap(), 0);
            assert!(r.remaining().is_empty());
            for (event, kind) in events[1..].iter().zip([6, 16]) {
                let mut r = Reader::new(event);
                assert_eq!(r.u32().unwrap(), 7);
                assert_eq!(r.u64().unwrap(), uid);
                assert_eq!(r.string(65).unwrap(), name);
                assert_eq!(r.u64().unwrap(), 123);
                let batch = r.blob().unwrap();
                assert!(batch.len() <= 1024);
                assert_eq!(
                    u32::from_le_bytes(batch[..4].try_into().unwrap()) as usize,
                    batch.len() - 4
                );
                // Broadcast manifold: 1+8+32+1+32 bits before its child.
                assert_eq!(
                    (u16::from_le_bytes(batch[13..15].try_into().unwrap()) >> 3) & 255,
                    kind
                );
                assert_eq!(r.u32().unwrap(), 1);
                assert!(r.remaining().is_empty());
            }
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
    fn bot_direct_delivery_is_scoped_and_requests_the_authoritative_roster_once() {
        let mut rooms = Rooms::default();
        let owner = session(42);
        let (id, _) = create(&mut rooms, &owner);
        rooms.channels.get_mut(&id).unwrap().bots_announced = true;
        let request = |recipient| {
            let mut w = Writer(vec![11, 3, 5]);
            w.u64(id);
            w.u64(recipient);
            // Native kind 5 JoinResponse, status zero (untyped bit stream).
            w.blob(&[6, 0, 0, 0, 10, 0, 0, 0, 0, 0]).unwrap();
            w.u32(1);
            w.0.push(0);
            w.0
        };
        let bytes = request(codol_protocol::room_messages::bot_id(11));
        let req = Request::parse(&bytes).unwrap();
        assert!(rooms.handle(&req, &session(43), 1).is_err());
        let unknown = request(123);
        assert!(
            rooms
                .handle(&Request::parse(&unknown).unwrap(), &owner, 1)
                .is_err()
        );
        let reply = rooms.handle(&req, &owner, 1).unwrap();
        assert!(reply.result.is_ok());
        assert_eq!(reply.pushes.len(), 2);
        let repeated = rooms.handle(&req, &owner, 1).unwrap();
        assert_eq!(repeated.pushes.len(), 1);
        let channel = rooms.channels.get_mut(&id).unwrap();
        assert!(channel.game_teams().is_err());
        channel.member_teams.insert(owner.user_id, 2); // Native room axis.
        for client in 1..=11 {
            channel.member_teams.insert(
                codol_protocol::room_messages::bot_id(client),
                if client % 2 == 0 { 2 } else { 1 },
            );
        }
        assert_eq!(
            channel.game_teams().unwrap(),
            [1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2]
        );
        channel.member_teams.insert(owner.user_id, 1);
        assert!(channel.game_teams().is_err()); // Seven allies cannot be a 6v6 roster.
    }
    #[test]
    fn human_readiness_rearms_bots_after_map_change_but_cancel_does_not_start_them() {
        let mut rooms = Rooms::default();
        let owner = session(42);
        let (id, _) = create(&mut rooms, &owner);
        rooms.channels.get_mut(&id).unwrap().bots_announced = true;
        for ready in [true, false, true] {
            let mut w = Writer(vec![11, 3, 4]);
            w.u64(id);
            w.blob(&[2, 0, 0, 0, 0x20, if ready { 2 } else { 0 }])
                .unwrap();
            w.u32(1);
            w.0.push(0);
            let result = rooms
                .handle(&Request::parse(&w.0).unwrap(), &owner, 1)
                .unwrap();
            assert_eq!(result.pushes.len(), if ready { 12 } else { 1 });
            for push in &result.pushes[1..] {
                let mut r = Reader::new(push);
                assert_eq!(r.u32().unwrap(), 7);
                r.u64().unwrap();
                r.string(65).unwrap();
                assert_eq!(r.u64().unwrap(), id);
                assert_eq!(
                    codol_protocol::room_messages::updates(r.blob().unwrap(), owner.user_id)
                        .unwrap()
                        .ready,
                    Some(true)
                );
            }
        }
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
    #[test]
    fn authenticated_session_renewal_preserves_only_its_own_room() {
        let mut rooms = Rooms::default();
        let mut owner = session(42);
        let (id, _) = create(&mut rooms, &owner);
        let (other, _) = create(&mut rooms, &session(43));
        owner.expires_at = 1000;
        let req = Request::parse(&[11, 3, 13, 0]).unwrap();
        let reply = rooms.handle(&req, &owner, 101).unwrap().result.unwrap();
        assert_eq!(Reader::new(&reply[0]).u64().unwrap(), id);
        assert!(!rooms.channels.contains_key(&other));
        assert_eq!(rooms.channels[&id].expires_at, 1000);
    }
}
