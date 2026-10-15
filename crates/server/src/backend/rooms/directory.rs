//! Native room browser snapshots, authenticated channel members and peer delivery.
use super::{Channel, Rooms};
use crate::backend::identity::session::Session;
use crate::contracts::lobby::DirectedPush;
use crate::contracts::lobby::Identity;
use crate::contracts::lobby::TaskResult;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::task;
use codol_protocol::lobby::task::Reader;
use codol_protocol::lobby::task::Request;
use codol_protocol::lobby::task::Writer;
use codol_protocol::room::browser::ChannelJoin;
use codol_protocol::room::browser::Position;
use codol_protocol::room::browser::Search;

pub(super) struct Human {
    pub identity: Identity,
    pub expires: u32,
    pub pending_until: Option<u32>,
    pub name: String,
    pub key: [u8; 24],
}
impl Human {
    pub fn owner(session: &Session) -> Self {
        Self {
            identity: session.into(),
            expires: session.expires_at,
            pending_until: None,
            name: STANDARD.encode(session.user_id.to_string()),
            key: session.key,
        }
    }
}
pub(super) struct Snapshot {
    criteria: [i32; 4],
    maximum: u32,
    expires: u32,
    entries: Vec<(u64, [u8; 8])>,
}
impl Channel {
    pub(super) fn is_owner(&self, session: &Session) -> bool {
        self.title_id == session.title_id
            && self.owner == session.user_id
            && self.session_seed == session.iv_seed
            && self
                .humans
                .get(&session.user_id)
                .is_some_and(|m| m.key == session.key)
    }
    pub(super) fn is_member(&self, session: &Session, now: u32) -> bool {
        self.title_id == session.title_id
            && self.humans.get(&session.user_id).is_some_and(|member| {
                member.identity == Identity::from(session)
                    && member.key == session.key
                    && member.expires > now
            })
    }
    pub(super) fn population(&self) -> u32 {
        self.humans.len() as u32
            + self.fill_bots.len() as u32
            + if self.bots_announced { 11 } else { 0 }
    }
    pub(super) fn info(&self, id: u64) -> Vec<u8> {
        let mut w = Writer::default();
        w.u64(id);
        w.u32(self.population());
        w.u32(0);
        w.u64(self.owner);
        w.boolean(false);
        w.boolean(false);
        w.u32(0);
        w.0
    }
    pub(super) fn membership_event(&self, id: u64, member: &Human, joined: bool) -> Vec<u8> {
        let mut w = Writer::default();
        w.u32(if joined { 9 } else { 10 });
        w.u64(member.identity.user);
        w.string(&member.name);
        w.u64(id);
        w.u8(2);
        w.u32(0);
        w.0.extend(self.info(id));
        w.0
    }
    fn targets(&self, actor: Identity, body: &[u8], result: &mut TaskResult) {
        for member in self.humans.values() {
            result.push_to(actor, member.identity, body.to_vec());
        }
    }
}

pub(super) fn chunks(batch: &[u8]) -> Result<Vec<Vec<u8>>> {
    ensure!(
        !batch.is_empty() && batch.len() <= 16384,
        "room message batch size"
    );
    let mut left = batch;
    let mut count = 0;
    let mut out = Vec::new();
    let mut current = Vec::new();
    while !left.is_empty() {
        let header = left
            .get(..4)
            .ok_or_else(|| anyhow::anyhow!("room message length missing"))?;
        let n = u32::from_le_bytes(header.try_into()?) as usize;
        ensure!(
            n > 0 && n <= left.len() - 4 && n + 4 <= 1024 && count < 64,
            "invalid room message framing"
        );
        if current.len() + 4 + n > 1024 {
            out.push(std::mem::take(&mut current));
        }
        current.extend_from_slice(&left[..4 + n]);
        left = &left[4 + n..];
        count += 1;
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}
fn message(
    sender: u64,
    name: &str,
    channel: u64,
    payload: &[u8],
    delivery: u32,
) -> Result<Vec<u8>> {
    let mut w = Writer::default();
    w.u32(7);
    w.u64(sender);
    w.string(name);
    w.u64(channel);
    w.blob(payload)?;
    w.u32(delivery);
    Ok(w.0)
}
impl Rooms {
    pub(crate) fn disconnect(&mut self, identity: Identity) -> Vec<DirectedPush> {
        self.snapshots.remove(&identity);
        let ids: Vec<_> = self
            .channels
            .iter()
            .filter(|(_, c)| {
                c.humans
                    .get(&identity.user)
                    .is_some_and(|m| m.identity == identity)
            })
            .map(|(id, _)| *id)
            .collect();
        let mut pushes = std::mem::take(&mut self.pending_events);
        for id in ids {
            if self.channels[&id].owner == identity.user {
                let channel = self.channels.remove(&id).expect("known channel");
                // A departed host closes the channel; migration is not fabricated.
                for target in channel.humans.values().filter(|m| m.identity != identity) {
                    for member in channel.humans.values() {
                        let mut empty = Writer::default();
                        empty.u32(10);
                        empty.u64(member.identity.user);
                        empty.string(&member.name);
                        empty.u64(id);
                        empty.u8(0);
                        pushes.push(DirectedPush {
                            target: target.identity,
                            body: empty.0,
                        });
                    }
                }
            } else {
                let channel = self.channels.get_mut(&id).expect("known channel");
                let member = channel.humans.remove(&identity.user).expect("known member");
                channel.member_teams.remove(&identity.user);
                let body = channel.membership_event(id, &member, false);
                for target in channel.humans.values() {
                    pushes.push(DirectedPush {
                        target: target.identity,
                        body: body.clone(),
                    });
                }
            }
        }
        if let Some(endpoint) = &self.game_endpoint {
            let session = Session {
                title_id: identity.title,
                user_id: identity.user,
                iv_seed: identity.seed,
                expires_at: 0,
                key: [0; 24],
            };
            endpoint.revoke(&session);
        }
        pushes
    }
    pub(crate) fn reap(&mut self, now: u32) -> Vec<DirectedPush> {
        self.snapshots.retain(|_, snapshot| snapshot.expires > now);
        let expired: Vec<_> = self
            .channels
            .values()
            .flat_map(|c| c.humans.values())
            .filter(|m| m.expires <= now || m.pending_until.is_some_and(|deadline| deadline <= now))
            .map(|m| m.identity)
            .collect();
        let mut pushes = std::mem::take(&mut self.pending_events);
        for identity in expired {
            pushes.extend(self.disconnect(identity));
        }
        pushes
    }
    pub(crate) fn discard_changed_credentials(&mut self, session: &Session) -> Vec<DirectedPush> {
        let identity = Identity::from(session);
        if self.channels.values().any(|c| {
            c.humans
                .get(&session.user_id)
                .is_some_and(|m| m.identity == identity && m.key != session.key)
        }) {
            self.disconnect(identity)
        } else {
            Vec::new()
        }
    }
    pub(super) fn browse(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let query = Search::parse(request)?;
        // Native creation publishes the same compatibility value in tail[0].
        // Its policy meaning is opaque; match it exactly rather than ignoring it.
        if query.criteria[1] != 0 || query.criteria[3] != 0 {
            return Ok(TaskResult::failure(
                "unsupported_browser_criteria",
                task::TASK_FAILED,
            ));
        }
        let identity = Identity::from(session);
        let fresh = matches!(
            query.position,
            Position::Offset(0) | Position::Page { initial: true, .. }
        );
        if fresh {
            if self.snapshots.len() >= 128 && !self.snapshots.contains_key(&identity) {
                return Ok(TaskResult::failure(
                    "browser_snapshot_capacity",
                    task::TASK_FAILED,
                ));
            }
            let mut entries: Vec<_> = self
                .channels
                .iter()
                .filter_map(|(id, c)| {
                    let game = c.game.as_ref()?;
                    (c.title_id == session.title_id
                        && c.expires_at > now
                        && game.description.build == query.criteria[0]
                        && game.description.compatibility == query.criteria[2])
                        .then_some((*id, game.id))
                })
                .collect();
            entries.sort_by_key(|(_, id)| *id);
            self.snapshots.insert(
                identity,
                Snapshot {
                    criteria: query.criteria,
                    maximum: query.maximum,
                    expires: now.saturating_add(60).min(session.expires_at),
                    entries,
                },
            );
        }
        let Some(snapshot) = self.snapshots.get(&identity).filter(|s| {
            s.criteria == query.criteria && s.maximum == query.maximum && s.expires > now
        }) else {
            return Ok(TaskResult::failure(
                "browser_snapshot_missing",
                task::TASK_FAILED,
            ));
        };
        let start = match query.position {
            Position::Offset(offset) => offset as usize,
            Position::Page { initial: true, .. } => 0,
            Position::Page {
                initial: false,
                cursor,
            } => match snapshot.entries.iter().position(|(_, id)| *id == cursor) {
                Some(index) => index + 1,
                None => {
                    return Ok(TaskResult::failure(
                        "browser_cursor_mismatch",
                        task::TASK_FAILED,
                    ));
                }
            },
        };
        let mut records = Vec::new();
        for (channel_id, id) in snapshot.entries.iter().skip(start) {
            if let Some(channel) = self.channels.get(channel_id)
                && let Some(game) = &channel.game
                && game.id == *id
                && channel.expires_at > now
                && game.description.build == query.criteria[0]
                && game.description.compatibility == query.criteria[2]
            {
                let listed = if self
                    .game_endpoint
                    .as_ref()
                    .is_some_and(|e| e.human_fill.is_some())
                {
                    channel.humans.len() as u32
                } else {
                    channel.population()
                };
                records.push(game.description.listing_record(id, listed)?);
                if records.len() == query.maximum as usize {
                    break;
                }
            }
        }
        Ok(TaskResult::success("public_room_search", records))
    }
    pub(super) fn join(
        &mut self,
        arguments: &[u8],
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let join = ChannelJoin::parse(arguments)?;
        if join.credential != 0 {
            return Ok(TaskResult::failure(
                "unsupported_channel_credential",
                task::TASK_FAILED,
            ));
        }
        let Some(channel) = self
            .channels
            .get(&join.channel_id)
            .filter(|c| c.title_id == session.title_id && c.game.is_some())
        else {
            return Ok(TaskResult::failure(
                "room_channel_missing",
                task::TASK_FAILED,
            ));
        };
        if channel.is_member(session, now) {
            return Ok(TaskResult::success("public_room_join_retry", vec![]));
        }
        if self
            .channels
            .values()
            .any(|c| c.humans.contains_key(&session.user_id) && c.title_id == session.title_id)
        {
            return Ok(TaskResult::failure("already_in_room", task::TASK_FAILED));
        }
        if self.game_endpoint.as_ref().is_some_and(|e| {
            e.room_started(channel.title_id, channel.owner, channel.session_seed, now)
        }) {
            return Ok(TaskResult::failure(
                "room_match_already_started",
                task::TASK_FAILED,
            ));
        }
        if channel.population()
            >= channel
                .game
                .as_ref()
                .expect("registered room")
                .description
                .maximum
            && channel.fill_bots.is_empty()
        {
            return Ok(TaskResult::failure(
                "room_capacity_reached",
                task::TASK_FAILED,
            ));
        }
        let channel = self
            .channels
            .get_mut(&join.channel_id)
            .expect("known channel");
        let mut member = Human::owner(session);
        if let Some(name) = self.names.get(&Identity::from(session)) {
            member.name = name.clone();
        }
        member.pending_until = Some(now.saturating_add(30));
        let mut result = TaskResult::success("public_room_channel_joined", vec![]);
        let actor = Identity::from(session);
        if let Some(client) = channel.fill_bots.iter().next_back().copied() {
            crate::backend::rooms::fill::remove_bot(
                channel,
                join.channel_id,
                client,
                actor,
                &mut result,
            );
        }
        channel.fill_sent.clear();
        channel.roster_requested = false;
        // The joiner's metadata follows its ACK; peers receive event9 before any
        // authenticated JoinRequest can be forwarded from this new member.
        channel.humans.insert(session.user_id, member);
        let body =
            channel.membership_event(join.channel_id, &channel.humans[&session.user_id], true);
        channel.targets(actor, &body, &mut result);
        Ok(result)
    }
    pub(super) fn leave(
        &mut self,
        arguments: &[u8],
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let mut r = Reader::new(arguments);
        let id = r.u64()?;
        r.finish()?;
        if !self
            .channels
            .get(&id)
            .is_some_and(|c| c.is_member(session, now))
        {
            return Ok(TaskResult::failure(
                "room_member_mismatch",
                task::TASK_FAILED,
            ));
        }
        let mut result = TaskResult::success("local_room_channel_left", vec![]);
        let actor = Identity::from(session);
        for push in self.disconnect(actor) {
            result.push_to(actor, push.target, push.body);
        }
        Ok(result)
    }
    pub(super) fn members(
        &self,
        arguments: &[u8],
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let mut r = Reader::new(arguments);
        let id = r.u64()?;
        let maximum = r.u32()?;
        r.finish()?;
        let Some(channel) = self
            .channels
            .get(&id)
            .filter(|c| c.is_member(session, now) && (1..=100).contains(&maximum))
        else {
            return Ok(TaskResult::failure(
                "room_member_mismatch",
                task::TASK_FAILED,
            ));
        };
        if channel.game.is_none() {
            return Ok(TaskResult::failure(
                "room_session_missing",
                task::TASK_FAILED,
            ));
        }
        let mut members: Vec<_> = channel.humans.values().collect();
        members.sort_by_key(|m| (m.identity.user != channel.owner, m.identity.user));
        let mut records = Vec::new();
        for member in members {
            if records.len() >= maximum as usize {
                break;
            }
            let mut w = Writer::default();
            w.u64(member.identity.user);
            w.string(&member.name);
            w.u32(0);
            records.push(w.0);
        }
        if channel.bots_announced {
            for client in 1..=11 {
                if records.len() >= maximum as usize {
                    break;
                }
                let mut w = Writer::default();
                w.u64(codol_protocol::room::messages::bot_id(client));
                w.string(&STANDARD.encode(codol_protocol::room::messages::bot_name(client)));
                w.u32(0);
                records.push(w.0);
            }
        }
        for client in &channel.fill_bots {
            if records.len() >= maximum as usize {
                break;
            }
            let mut w = Writer::default();
            w.u64(codol_protocol::room::messages::bot_id(*client));
            w.string(&STANDARD.encode(codol_protocol::room::messages::bot_name(*client)));
            w.u32(0);
            records.push(w.0);
        }
        Ok(TaskResult::success("local_room_members", records))
    }
    pub(super) fn peer_send(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let mut r = Reader::new(request.arguments);
        let id = r.u64()?;
        let recipient = if request.task == 5 {
            Some(r.u64()?)
        } else {
            None
        };
        let batch = r.blob()?;
        let delivery = r.u32()?;
        r.finish()?;
        let payloads = chunks(batch)?;
        let Some(channel) = self
            .channels
            .get_mut(&id)
            .filter(|c| c.is_member(session, now))
        else {
            return Ok(TaskResult::failure(
                "room_member_mismatch",
                task::TASK_FAILED,
            ));
        };
        if let Some(recipient) = recipient {
            ensure!(
                channel.humans.contains_key(&recipient)
                    || channel
                        .fill_bots
                        .iter()
                        .any(|c| codol_protocol::room::messages::bot_id(*c) == recipient),
                "recipient is not a channel member"
            );
        }
        let owner = channel.is_owner(session);
        if !owner {
            codol_protocol::room::messages::validate_participant(batch, channel.owner)?;
        }
        let mut result = TaskResult::success(
            if recipient.is_some() {
                "public_room_direct_message"
            } else {
                "public_room_broadcast"
            },
            vec![],
        );
        let actor = Identity::from(session);
        let name = &channel.humans[&session.user_id].name;
        for payload in payloads {
            let body = message(session.user_id, name, id, &payload, delivery)?;
            if let Some(recipient) = recipient {
                if let Some(human) = channel.humans.get(&recipient) {
                    result.push_to(actor, human.identity, body);
                }
            } else {
                channel.targets(actor, &body, &mut result);
            }
        }
        if owner {
            let recipients: Vec<_> = channel
                .humans
                .keys()
                .copied()
                .chain(
                    channel
                        .fill_bots
                        .iter()
                        .map(|c| codol_protocol::room::messages::bot_id(*c)),
                )
                .collect();
            let updates = codol_protocol::room::messages::channel_updates(batch, &recipients)?;
            channel.update_members(&updates)?;
            if updates.full_roster
                && !self.game_endpoint.as_ref().is_some_and(|e| {
                    e.room_started(channel.title_id, channel.owner, channel.session_seed, now)
                })
            {
                let removed: Vec<_> = channel
                    .fill_bots
                    .iter()
                    .copied()
                    .filter(|c| {
                        !channel
                            .member_teams
                            .contains_key(&codol_protocol::room::messages::bot_id(*c))
                    })
                    .collect();
                for client in removed {
                    super::fill::remove_bot(channel, id, client, actor, &mut result);
                    channel.roster_requested = false;
                }
            }
            for member in &updates.members {
                if let Some(human) = channel.humans.get_mut(&member.id) {
                    human.pending_until = None;
                }
            }
            if let Some(info) = updates.info {
                channel.room_info = Some(info);
                channel.owner_ready = false;
            }
            if let Some(ready) = updates.ready {
                channel.owner_ready = ready;
            }
            let destinations: Vec<_> = match recipient {
                Some(recipient) => channel
                    .humans
                    .contains_key(&recipient)
                    .then_some(recipient)
                    .into_iter()
                    .collect(),
                None => channel
                    .humans
                    .keys()
                    .copied()
                    .filter(|uid| *uid != channel.owner)
                    .collect(),
            };
            for recipient in destinations {
                let updates = codol_protocol::room::messages::updates(batch, recipient)?;
                if let Some(status) = updates.join_result {
                    if status == 0 {
                        channel
                            .humans
                            .get_mut(&recipient)
                            .expect("validated recipient")
                            .pending_until = None;
                    } else {
                        let member = channel
                            .humans
                            .remove(&recipient)
                            .expect("validated recipient");
                        channel.member_teams.remove(&recipient);
                        let body = channel.membership_event(id, &member, false);
                        channel.targets(actor, &body, &mut result);
                        result.push_to(actor, member.identity, body);
                    }
                }
            }
        }
        if let Some(fill) = self
            .game_endpoint
            .as_ref()
            .filter(|e| !e.room_started(channel.title_id, channel.owner, channel.session_seed, now))
            .and_then(|e| e.human_fill.as_ref())
        {
            crate::backend::rooms::fill::sync(channel, id, fill, actor, &mut result)?;
        }
        Ok(result)
    }
}
