//! Authenticated room channels and native peer delivery. Gameplay is separate.
mod directory;
#[cfg(test)]
mod directory_tests;
use crate::backend::identity::session::Session;
use crate::contracts::lobby::Identity;
use crate::contracts::lobby::TaskResult;
#[cfg(test)]
mod story_tests;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::task;
use codol_protocol::lobby::task::Reader;
use codol_protocol::lobby::task::Request;
use codol_protocol::lobby::task::Writer;
use codol_protocol::room::description::Description;
use directory::{Human, Snapshot};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

struct Channel {
    title_id: u32,
    owner: u64,
    session_seed: u32,
    expires_at: u32,
    game: Option<GameSession>,
    room_info: Option<codol_protocol::room::messages::RoomInfo>,
    messages: VecDeque<Vec<u8>>,
    bots_announced: bool,
    owner_ready: bool,
    roster_requested: bool,
    member_teams: HashMap<u64, u8>,
    avatar_sent: HashMap<u8, (String, String, u8, codol_protocol::room::messages::Loadout)>,
    humans: HashMap<u64, Human>,
}
struct GameSession {
    id: [u8; 8],
    description: Description,
}
fn bot_message(client: u8, channel_id: u64, owner: u64, record: &[u8]) -> Result<Vec<u8>> {
    let mut push = Writer::default();
    push.u32(7);
    push.u64(codol_protocol::room::messages::bot_id(client));
    push.string(&STANDARD.encode(codol_protocol::room::messages::bot_name(client)));
    push.u64(channel_id);
    push.blob(&codol_protocol::room::messages::to_member(record, owner)?)?;
    push.u32(1);
    Ok(push.0)
}
impl Channel {
    fn bot_selection<'a>(
        &'a self,
        policy: &'a crate::backend::rooms::bots::Policy,
    ) -> Option<&'a (String, String)> {
        let description = &self.game.as_ref()?.description;
        if description.build != -688906341 || description.maximum != 12 {
            return None;
        }
        if let Some(info) = &self.room_info {
            return policy
                .accepts(info)
                .then(|| policy.maps.get(&(info.map.clone(), info.mode.clone())))
                .flatten();
        }
        // Creation need not broadcast kind15. The authenticated descriptor
        // already identifies the custom map/mode through their native CRCs.
        policy
            .maps
            .iter()
            .find(|(map, mode)| description.matches_custom_map(map, mode))
    }
    fn update_members(&mut self, updates: &codol_protocol::room::messages::Updates) -> Result<()> {
        let mut candidate = if updates.full_roster {
            HashMap::new()
        } else {
            self.member_teams.clone()
        };
        for member in &updates.members {
            anyhow::ensure!(
                self.humans.contains_key(&member.id)
                    || (self.bots_announced
                        && (1..=11)
                            .any(|c| codol_protocol::room::messages::bot_id(c) == member.id)),
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
                codol_protocol::room::messages::bot_id(client as u8)
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
    policy: &crate::backend::rooms::bots::Policy,
    channel_id: u64,
    session: &Session,
    ready: bool,
) -> Result<Vec<Vec<u8>>> {
    let mut pushes = Vec::new();
    for client in 1..=codol_protocol::room::messages::BOT_CLIENTS {
        let uid = codol_protocol::room::messages::bot_id(client);
        anyhow::ensure!(uid != session.user_id, "bot identity overlaps room owner");
        // 0x6d3830 Base64-decodes SDK names before publishing room members.
        // Match snapshots use the plain display name through a separate codec.
        let name = STANDARD.encode(codol_protocol::room::messages::bot_name(client));
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
            codol_protocol::room::messages::bot_join(client, policy.version)?,
            codol_protocol::room::messages::bot_ready(ready),
        ] {
            let payload = codol_protocol::room::messages::to_member(&record, session.user_id)?;
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
    game_endpoint: Option<Arc<crate::backend::rooms::admission::GameAdmissions>>,
    snapshots: HashMap<Identity, Snapshot>,
    pending_events: Vec<crate::contracts::lobby::DirectedPush>,
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
    pub fn with_game_endpoint(
        endpoint: Arc<crate::backend::rooms::admission::GameAdmissions>,
    ) -> Self {
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
                let query = codol_protocol::room::description::Search::parse(request.arguments)?;
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
                        let record = codol_protocol::room::story::allocation_record(
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
                            avatar_sent: HashMap::new(),
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
                    .find(|&c| codol_protocol::room::messages::bot_id(c) == recipient)
                    .ok_or_else(|| anyhow::anyhow!("recipient is not a local room bot"))?;
                anyhow::ensure!(delivery <= 1 && !batch.is_empty(), "invalid bot delivery");
                let updates = codol_protocol::room::messages::updates(batch, recipient)?;
                channel.update_members(&updates)?;
                let mut result = TaskResult::success("local_room_bot_delivery", vec![]);
                if updates.join_result == Some(0) {
                    result.pushes.push(bot_message(
                        client,
                        id,
                        session.user_id,
                        &codol_protocol::room::messages::bot_ready(channel.owner_ready),
                    )?);
                    if client == 11 && !channel.roster_requested {
                        result.pushes.push(bot_message(
                            client,
                            id,
                            session.user_id,
                            &codol_protocol::room::messages::roster_request(),
                        )?);
                        channel.roster_requested = true;
                    }
                }
                if updates.full_roster {
                    tracing::info!(teams=?channel.game_teams()?, "native bot room roster confirmed");
                }
                if let Some(policy) = self
                    .game_endpoint
                    .as_ref()
                    .and_then(|e| e.bot_rooms.as_ref())
                {
                    result.pushes.extend(channel.sync_bot_loadouts(
                        policy,
                        id,
                        updates.avatar_refresh,
                    )?);
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
                    codol_protocol::room::messages::bot_room_updates(batch, session.user_id)?
                } else {
                    Default::default()
                };
                let avatar_reset = updates.info.is_some()
                    || updates.ready == Some(false)
                    || updates.avatar_refresh;
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
                    for client in 1..=codol_protocol::room::messages::BOT_CLIENTS {
                        result.pushes.push(bot_message(
                            client,
                            id,
                            session.user_id,
                            &codol_protocol::room::messages::bot_ready(channel.owner_ready),
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
                    && let Some((map, mode)) = channel.bot_selection(policy).cloned()
                {
                    result
                        .pushes
                        .extend(bot_pushes(policy, id, session, channel.owner_ready)?);
                    channel.bots_announced = true;
                    tracing::info!(map, mode, bots = 11, "native room bot participation queued");
                }
                if let Some(policy) = self
                    .game_endpoint
                    .as_ref()
                    .and_then(|e| e.bot_rooms.as_ref())
                {
                    result
                        .pushes
                        .extend(channel.sync_bot_loadouts(policy, id, avatar_reset)?);
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
mod tests;

pub(crate) mod bots;

pub mod admission;
