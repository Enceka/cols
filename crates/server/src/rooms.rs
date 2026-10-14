//! Authenticated room channels and native peer delivery. Gameplay is separate.
#[path = "room_directory.rs"]
mod directory;
#[cfg(test)]
#[path = "room_directory_tests.rs"]
mod directory_tests;
use crate::lobby_delivery::Identity;
use crate::{services::TaskResult, session::Session};
#[cfg(test)]
#[path = "story_room_tests.rs"]
mod story_tests;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::room::Description;
use codol_protocol::task::{self, Reader, Request, Writer};
use directory::{Human, Snapshot};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

struct Channel {
    title_id: u32,
    owner: u64,
    session_seed: u32,
    expires_at: u32,
    game: Option<GameSession>,
    room_info: Option<codol_protocol::room_messages::RoomInfo>,
    messages: VecDeque<Vec<u8>>,
    bots_announced: bool,
    owner_ready: bool,
    roster_requested: bool,
    member_teams: HashMap<u64, u8>,
    humans: HashMap<u64, Human>,
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
    fn bot_map<'a>(&'a self, policy: &'a crate::room_bots::Policy) -> Option<&'a str> {
        let description = &self.game.as_ref()?.description;
        if description.build != -688906341 || description.maximum != 12 {
            return None;
        }
        if let Some(info) = &self.room_info {
            return policy.accepts(info).then_some(info.map.as_str());
        }
        // Creation need not broadcast kind15. The authenticated descriptor
        // already identifies the custom map/mode through their native CRCs.
        policy
            .maps
            .iter()
            .find(|map| description.matches_custom_map(map, "war"))
            .map(String::as_str)
    }
    fn update_members(&mut self, updates: &codol_protocol::room_messages::Updates) -> Result<()> {
        let mut candidate = if updates.full_roster {
            HashMap::new()
        } else {
            self.member_teams.clone()
        };
        for member in &updates.members {
            anyhow::ensure!(
                self.humans.contains_key(&member.id)
                    || (self.bots_announced
                        && (1..=11).any(|c| codol_protocol::room_messages::bot_id(c) == member.id)),
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
    ready: bool,
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
            codol_protocol::room_messages::bot_ready(ready),
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
    snapshots: HashMap<Identity, Snapshot>,
    pending_events: Vec<crate::lobby_delivery::DirectedPush>,
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
        for channel in self.channels.values_mut() {
            if let Some(member) = channel.humans.get_mut(&session.user_id)
                && member.identity == Identity::from(session)
                && member.key == session.key
                && member.expires > now
            {
                member.expires = session.expires_at;
            }
        }
        Ok(())
    }
    pub fn with_game_endpoint(endpoint: Arc<crate::game_server::GameAdmissions>) -> Self {
        Self {
            channels: HashMap::new(),
            game_endpoint: Some(endpoint),
            snapshots: HashMap::new(),
            pending_events: Vec::new(),
        }
    }
    pub fn handle(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let removed = self.reap(now);
        if session.expires_at <= now {
            let mut response = TaskResult::failure("expired_room_session", task::TASK_FAILED);
            for push in removed {
                response.push_to(session.into(), push.target, push.body);
            }
            return Ok(response);
        }
        self.renew_session(session, now)?;
        let mut response = match self.handle_live(request, session, now) {
            Ok(response) => response,
            Err(error) => {
                self.pending_events.extend(removed);
                return Err(error);
            }
        };
        for push in removed {
            response.push_to(session.into(), push.target, push.body);
        }
        Ok(response)
    }
    fn handle_live(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        if request.service == 21
            && (request.task == 13
                || (request.task == 5 && Reader::new(request.arguments).u32()? == 4))
        {
            return self.browse(request, session, now);
        }
        if request.service == 11 {
            match request.task {
                11 => return self.join(request.arguments, session, now),
                2 => return self.leave(request.arguments, session, now),
                15 => return self.members(request.arguments, session, now),
                4 | 5 => {
                    let mut r = Reader::new(request.arguments);
                    let id = r.u64()?;
                    let human_target = request.task == 5
                        && self.channels.get(&id).is_some_and(|c| {
                            r.u64().ok().is_some_and(|uid| c.humans.contains_key(&uid))
                        });
                    if self
                        .channels
                        .get(&id)
                        .is_some_and(|c| !c.is_owner(session) || c.humans.len() > 1)
                        || human_target
                    {
                        return self.peer_send(request, session, now);
                    }
                }
                _ => {}
            }
        }
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
                let Some((channel_id, channel)) = self.channels.iter().find(|(_, c)| {
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
                if channel.humans.len() != 1 {
                    return Ok(TaskResult::failure(
                        "shared_game_world_not_available",
                        task::TASK_FAILED,
                    ));
                }
                let Some(game) = channel.game.as_ref() else {
                    if endpoint.story_capture_map.is_some() {
                        let (id, key) = endpoint.allocate_story(
                            session,
                            *channel_id,
                            query.criteria[0],
                            now,
                        )?;
                        let record = codol_protocol::story::allocation_record(
                            endpoint.address(),
                            *channel_id,
                            query.criteria[0],
                            &id,
                            &key,
                        )?;
                        return Ok(TaskResult::success(
                            "local_story_transport_allocated",
                            vec![record],
                        ));
                    }
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
                if self
                    .channels
                    .values()
                    .any(|c| c.is_member(session, now) && !c.is_owner(session))
                {
                    return Ok(TaskResult::failure("already_in_room", task::TASK_FAILED));
                }
                if let Some(endpoint) = &self.game_endpoint {
                    endpoint.revoke_other_sessions(session);
                }
                let old: Vec<_> = self
                    .channels
                    .values()
                    .flat_map(|c| c.humans.values())
                    .filter(|m| {
                        m.identity.title == session.title_id
                            && m.identity.user == session.user_id
                            && m.identity.seed != session.iv_seed
                    })
                    .map(|m| m.identity)
                    .collect();
                for identity in old {
                    let pushes = self.disconnect(identity);
                    self.pending_events.extend(pushes);
                }
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
                            room_info: None,
                            messages: VecDeque::new(),
                            bots_announced: false,
                            owner_ready: false,
                            roster_requested: false,
                            member_teams: HashMap::new(),
                            humans: HashMap::from([(session.user_id, Human::owner(session))]),
                        },
                    );
                    id
                };
                // Target bdChannelInfo decoder 0xa55a40, bound by 0xa4c940.
                let mut record = Writer::default();
                record.u64(id);
                record.u32(self.channels[&id].population());
                record.u32(0);
                record.u64(session.user_id);
                record.boolean(false);
                record.boolean(false);
                record.u32(0);
                let mut response =
                    TaskResult::success("local_room_channel_created", vec![record.0]);
                for push in std::mem::take(&mut self.pending_events) {
                    response.push_to(session.into(), push.target, push.body);
                }
                Ok(response)
            }
            (21, 1) => {
                let description = Description::parse(request.arguments, session.user_id)?;
                ensure!(
                    STANDARD.encode(&description.owner_name).len() < 64,
                    "SDK owner name exceeds capacity"
                );
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
                    channel.room_info = None;
                    let mut id = [0u8; 8];
                    openssl::rand::rand_bytes(&mut id)?;
                    id
                };
                channel.game = Some(GameSession { id, description });
                channel
                    .humans
                    .get_mut(&session.user_id)
                    .expect("owner membership")
                    .name = STANDARD.encode(
                    &channel
                        .game
                        .as_ref()
                        .expect("registered")
                        .description
                        .owner_name,
                );
                let mut record = Writer::default();
                record.blob(&id)?;
                // Wait for the owner's first broadcast: creation's RPC
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
                ensure!(
                    STANDARD.encode(&description.owner_name).len() < 64,
                    "SDK owner name exceeds capacity"
                );
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
                channel
                    .humans
                    .get_mut(&session.user_id)
                    .expect("owner membership")
                    .name = STANDARD.encode(&game.description.owner_name);
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
                channel.room_info = None;
                if let Some(endpoint) = &self.game_endpoint {
                    endpoint.revoke(session);
                }
                Ok(TaskResult::success("local_room_session_deleted", vec![]))
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
                        &codol_protocol::room_messages::bot_ready(channel.owner_ready),
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
                    push.string(&STANDARD.encode(name));
                    push.u64(id);
                    push.blob(&payload)?;
                    push.u32(delivery);
                    result.pushes.push(push.0);
                }
                // Map changes and returning from a match reset native ready
                // flags. The human chooses when to start the next round;
                // simulated peers then rejoin readiness for that room state.
                if updates.info.is_some() {
                    channel.owner_ready = false;
                }
                if let Some(ready) = updates.ready {
                    channel.owner_ready = ready;
                }
                if channel.bots_announced && (updates.ready.is_some() || updates.info.is_some()) {
                    for client in 1..=codol_protocol::room_messages::BOT_CLIENTS {
                        result.pushes.push(bot_message(
                            client,
                            id,
                            session.user_id,
                            &codol_protocol::room_messages::bot_ready(channel.owner_ready),
                        )?);
                    }
                }
                if let Some(info) = updates.info {
                    channel.room_info = Some(info);
                }
                if !channel.bots_announced
                    && let Some(policy) = self
                        .game_endpoint
                        .as_ref()
                        .and_then(|e| e.bot_rooms.as_ref())
                    && let Some(map) = channel.bot_map(policy).map(str::to_owned)
                {
                    result
                        .pushes
                        .extend(bot_pushes(policy, id, session, channel.owner_ready)?);
                    channel.bots_announced = true;
                    tracing::info!(
                        map,
                        mode = "war",
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
        let pushes = bot_pushes(&policy, 123, &owner, false).unwrap();
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
                if kind == 16 {
                    assert_eq!(
                        codol_protocol::room_messages::updates(batch, owner.user_id)
                            .unwrap()
                            .ready,
                        Some(false)
                    );
                }
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
    #[tokio::test]
    async fn supported_creation_joins_bots_after_receiver_ready_without_a_map_change() {
        let temp = tempfile::tempdir().unwrap();
        let server = crate::game_server::GameServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            temp.path().join("game"),
        )
        .await
        .unwrap();
        let mut endpoint = server.admissions();
        drop(server);
        Arc::get_mut(&mut endpoint).unwrap().bot_rooms = Some(crate::room_bots::Policy {
            version: 7,
            maps: ["mp_killhouse_sh".into()].into(),
        });
        for (map, mode, playlist, capacity, explicit_unsupported, expected_joins) in [
            ("mp_killhouse_sh", "war", u32::MAX, 12, false, 33),
            ("mp_highrise", "war", u32::MAX, 12, false, 0),
            ("mp_killhouse_sh", "sd", u32::MAX, 12, false, 0),
            ("mp_killhouse_sh", "war", 0, 12, false, 0),
            ("mp_killhouse_sh", "war", u32::MAX, 18, false, 0),
            ("mp_killhouse_sh", "war", u32::MAX, 12, true, 0),
        ] {
            let mut rooms = Rooms::with_game_endpoint(endpoint.clone());
            let owner = session(0x8000000000000001);
            let (channel_id, _) = create(&mut rooms, &owner);
            let mut description =
                include_bytes!("../../../fixtures/protocol/room-create-synthetic.bin").to_vec();
            let n = description.len();
            description[n - 19..n - 11].copy_from_slice(&channel_id.to_le_bytes());
            for (attribute, value) in [
                (4, crc32fast::hash(map.as_bytes())),
                (5, playlist),
                (10, crc32fast::hash(mode.as_bytes())),
            ] {
                let offset = n - 74 + attribute * 5;
                description[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            assert_eq!(
                Description::parse(&description, owner.user_id)
                    .unwrap()
                    .matches_custom_map(map, mode),
                playlist == u32::MAX
            );
            let mut create_game = vec![21, 3, 1];
            create_game.extend(description);
            let result = rooms
                .handle(&Request::parse(&create_game).unwrap(), &owner, 1)
                .unwrap();
            assert!(result.result.is_ok());
            assert!(result.pushes.is_empty()); // Receiver is not installed yet.
            rooms
                .channels
                .get_mut(&channel_id)
                .unwrap()
                .game
                .as_mut()
                .unwrap()
                .description
                .maximum = capacity;
            let broadcast = |batch: &[u8]| {
                let mut w = Writer(vec![11, 3, 4]);
                w.u64(channel_id);
                w.blob(batch).unwrap();
                w.u32(1);
                w.0.push(0);
                w.0
            };
            if explicit_unsupported {
                // A newer kind15 choice overrides a still-old creation CRC.
                let payload = b"war\0mp_highrise\0\xff\x04\x0c";
                let mut record = vec![0; payload.len() + 2];
                record[0] = 15 << 1;
                for (i, b) in payload.iter().enumerate() {
                    record[i + 1] |= b << 1;
                    record[i + 2] |= b >> 7;
                }
                let mut batch = (record.len() as u32).to_le_bytes().to_vec();
                batch.extend(record);
                let result = rooms
                    .handle(&Request::parse(&broadcast(&batch)).unwrap(), &owner, 1)
                    .unwrap();
                assert_eq!(result.pushes.len(), 1);
            }
            let message = broadcast(&[2, 0, 0, 0, 0x20, 0]);
            let request = Request::parse(&message).unwrap();
            assert!(
                rooms
                    .handle(&request, &session(2), 1)
                    .unwrap()
                    .result
                    .is_err()
            );
            let result = rooms.handle(&request, &owner, 1).unwrap();
            assert_eq!(result.pushes.len(), 1 + expected_joins, "{map}");
            assert_eq!(
                rooms.handle(&request, &owner, 1).unwrap().pushes.len(),
                if expected_joins > 0 { 12 } else { 1 }
            );
        }
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
        let denied = rooms.handle(&req, &session(43), 1).unwrap();
        assert!(denied.result.is_err());
        assert!(denied.pushes.is_empty() && denied.deliveries.is_empty());
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
            assert_eq!(result.pushes.len(), 12);
            assert_eq!(rooms.channels[&id].owner_ready, ready);
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
                    Some(ready)
                );
            }
        }
    }
    #[test]
    fn map_selection_clears_readiness_before_a_delayed_bot_join_reply() {
        let mut rooms = Rooms::default();
        let owner = session(42);
        let (id, _) = create(&mut rooms, &owner);
        let channel = rooms.channels.get_mut(&id).unwrap();
        channel.bots_announced = true;
        channel.owner_ready = true;
        let payload = b"war\0mp_storm_spring\0\xff\x04\x0c";
        let mut record = vec![0; payload.len() + 2];
        record[0] = 15 << 1;
        for (i, b) in payload.iter().enumerate() {
            record[i + 1] |= b << 1;
            record[i + 2] |= b >> 7;
        }
        let mut batch = (record.len() as u32).to_le_bytes().to_vec();
        batch.extend(record);
        let mut broadcast = Writer(vec![11, 3, 4]);
        broadcast.u64(id);
        broadcast.blob(&batch).unwrap();
        broadcast.u32(1);
        broadcast.0.push(0);
        let result = rooms
            .handle(&Request::parse(&broadcast.0).unwrap(), &owner, 1)
            .unwrap();
        assert_eq!(result.pushes.len(), 12);
        assert!(!rooms.channels[&id].owner_ready);
        let mut direct = Writer(vec![11, 3, 5]);
        direct.u64(id);
        direct.u64(codol_protocol::room_messages::bot_id(1));
        direct.blob(&[6, 0, 0, 0, 10, 0, 0, 0, 0, 0]).unwrap();
        direct.u32(1);
        direct.0.push(0);
        let reply = rooms
            .handle(&Request::parse(&direct.0).unwrap(), &owner, 1)
            .unwrap();
        assert_eq!(reply.pushes.len(), 1);
        for push in result.pushes[1..].iter().chain(&reply.pushes) {
            let mut r = Reader::new(push);
            assert_eq!(r.u32().unwrap(), 7);
            r.u64().unwrap();
            r.string(65).unwrap();
            assert_eq!(r.u64().unwrap(), id);
            assert_eq!(
                codol_protocol::room_messages::updates(r.blob().unwrap(), owner.user_id)
                    .unwrap()
                    .ready,
                Some(false)
            );
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
        assert_eq!(reader.string(65).unwrap(), "TGFiIHBsYXllcg==");
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
        let mut authenticated = a.clone();
        authenticated.expires_at = 1000;
        assert!(rooms.handle(&bad, &authenticated, 101).is_err());
        assert!(rooms.channels.is_empty());
    }
    #[test]
    fn authenticated_session_renewal_preserves_only_its_own_room() {
        let mut rooms = Rooms::default();
        let mut owner = session(42);
        let (id, _) = create(&mut rooms, &owner);
        let (other, _) = create(&mut rooms, &session(43));
        owner.expires_at = 1000;
        rooms.renew_session(&owner, 99).unwrap();
        let req = Request::parse(&[11, 3, 13, 0]).unwrap();
        let reply = rooms.handle(&req, &owner, 101).unwrap().result.unwrap();
        assert_eq!(Reader::new(&reply[0]).u64().unwrap(), id);
        assert!(!rooms.channels.contains_key(&other));
        assert_eq!(rooms.channels[&id].expires_at, 1000);
    }
}
