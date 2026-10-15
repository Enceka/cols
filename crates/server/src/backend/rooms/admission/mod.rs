//! Session-bound game allocation and booking, independent of UDP listeners.
pub(crate) mod story_capture;
use crate::backend::identity::session::Session;
use crate::contracts::lobby::TaskResult;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::task::{self, Reader, Writer};
use std::{collections::HashMap, net::SocketAddrV4, sync::Mutex};
#[derive(Clone)]
pub(crate) struct Admission {
    pub(crate) title: u32,
    pub(crate) owner: u64,
    pub(crate) seed: u32,
    pub(crate) build: i32,
    pub(crate) expires: u32,
    pub(crate) key: [u8; 16],
    pub(crate) booked: bool,
    pub(crate) finished: bool,
    pub(crate) playlist_version: Option<u32>,
    pub(crate) start_command: Option<String>,
    pub(crate) room_teams: Option<[u8; 12]>,
    pub(crate) story_channel: Option<u64>,
    pub(crate) participants: Vec<crate::contracts::match_session::Participant>,
}

pub struct GameAdmissions {
    address: SocketAddrV4,
    entries: Mutex<HashMap<[u8; 8], Admission>>,
    pub(crate) bot_rooms: Option<crate::backend::rooms::bots::Policy>,
    pub(crate) human_fill: Option<crate::backend::rooms::bots::FillPolicy>,
    pub(crate) story_capture_map: Option<String>,
}
impl GameAdmissions {
    pub(crate) fn new(address: SocketAddrV4) -> Self {
        Self {
            address,
            entries: Mutex::new(HashMap::new()),
            bot_rooms: None,
            human_fill: None,
            story_capture_map: None,
        }
    }
    pub(crate) fn has_live_entries(&self, now: u32) -> Result<bool> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?
            .values()
            .any(|a| a.expires > now))
    }

    pub(crate) fn room_started(&self, title: u32, owner: u64, seed: u32, now: u32) -> bool {
        self.entries.lock().is_ok_and(|entries| {
            entries.values().any(|a| {
                a.title == title
                    && a.owner == owner
                    && a.seed == seed
                    && a.expires > now
                    && a.start_command.is_some()
                    && !a.finished
            })
        })
    }
    pub(crate) fn finish(&self, id: [u8; 8]) -> Result<()> {
        if let Some(entry) = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?
            .get_mut(&id)
        {
            entry.finished = true;
        }
        Ok(())
    }
    pub fn address(&self) -> SocketAddrV4 {
        self.address
    }
    pub(crate) fn allocate(
        &self,
        session: &Session,
        security_key: [u8; 16],
        build: i32,
        now: u32,
    ) -> Result<([u8; 8], [u8; 16])> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        ensure!(session.expires_at > now, "expired game allocation session");
        entries.retain(|_, a| {
            a.expires > now
                && (a.title != session.title_id
                    || a.owner != session.user_id
                    || (a.seed == session.iv_seed
                        && a.key == security_key
                        && a.build == build
                        && a.story_channel.is_none()
                        && a.start_command.is_none()))
        });
        if let Some((id, a)) = entries.iter().find(|(_, a)| {
            a.title == session.title_id && a.owner == session.user_id && a.seed == session.iv_seed
        }) {
            return Ok((*id, a.key));
        }
        ensure!(entries.len() < 32, "game admission capacity");
        let mut id = [0u8; 8];
        let key = security_key;
        loop {
            openssl::rand::rand_bytes(&mut id)?;
            if id != [0; 8] && !entries.contains_key(&id) {
                break;
            }
        }
        entries.insert(
            id,
            Admission {
                title: session.title_id,
                owner: session.user_id,
                seed: session.iv_seed,
                build,
                expires: session.expires_at.min(now.saturating_add(930)),
                key,
                booked: false,
                finished: false,
                playlist_version: None,
                start_command: None,
                room_teams: None,
                story_channel: None,
                participants: Vec::new(),
            },
        );
        Ok((id, key))
    }
    /// Build-only story searches do not upload a client key. The opt-in
    /// endpoint owns one random key per live channel and login identity.
    pub(crate) fn allocate_story(
        &self,
        session: &Session,
        channel: u64,
        build: i32,
        now: u32,
    ) -> Result<([u8; 8], [u8; 16])> {
        ensure!(
            self.story_capture_map.as_deref() == Some(codol_protocol::room::story::FIRST_MAP),
            "story capture is not enabled"
        );
        ensure!(
            session.expires_at > now
                && !matches!(channel, 0 | u64::MAX)
                && build == codol_protocol::room::story::CLIENT_BUILD,
            "invalid story allocation identity"
        );
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        entries.retain(|_, a| {
            a.expires > now
                && (a.title != session.title_id
                    || a.owner != session.user_id
                    || (a.seed == session.iv_seed
                        && a.build == build
                        && a.story_channel == Some(channel)))
        });
        if let Some((id, a)) = entries.iter().find(|(_, a)| {
            a.title == session.title_id && a.owner == session.user_id && a.seed == session.iv_seed
        }) {
            return Ok((*id, a.key));
        }
        ensure!(entries.len() < 32, "game admission capacity");
        let mut id = [0; 8];
        loop {
            openssl::rand::rand_bytes(&mut id)?;
            if id != [0; 8] && !entries.contains_key(&id) {
                break;
            }
        }
        let mut key = [0; 16];
        loop {
            openssl::rand::rand_bytes(&mut key)?;
            if key != [0; 16] {
                break;
            }
        }
        entries.insert(
            id,
            Admission {
                title: session.title_id,
                owner: session.user_id,
                seed: session.iv_seed,
                build,
                expires: session.expires_at.min(now.saturating_add(930)),
                key,
                booked: false,
                finished: false,
                playlist_version: None,
                start_command: None,
                room_teams: None,
                story_channel: Some(channel),
                participants: Vec::new(),
            },
        );
        Ok((id, key))
    }
    pub(crate) fn set_room_teams(&self, session: &Session, teams: [u8; 12]) -> Result<()> {
        ensure!(
            teams.iter().all(|t| matches!(t, 1 | 2))
                && teams.iter().filter(|&&t| t == 1).count() == 6,
            "invalid native match roster"
        );
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        let entry = entries
            .values_mut()
            .find(|a| {
                a.title == session.title_id
                    && a.owner == session.user_id
                    && a.seed == session.iv_seed
            })
            .ok_or_else(|| anyhow::anyhow!("room roster requires allocated game"))?;
        entry.room_teams = Some(teams);
        Ok(())
    }
    pub(crate) fn set_participants(
        &self,
        id: [u8; 8],
        participants: Vec<crate::contracts::match_session::Participant>,
    ) -> Result<()> {
        ensure!(
            (2..=12).contains(&participants.len()),
            "invalid human match size"
        );
        ensure!(
            participants.iter().all(|p| matches!(p.team, 1 | 2))
                && participants
                    .iter()
                    .map(|p| p.user_id)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    == participants.len(),
            "invalid human match membership"
        );
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        let entry = entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("missing room allocation"))?;
        ensure!(
            !entry.booked || entry.participants == participants,
            "match membership already frozen"
        );
        entry.participants = participants;
        Ok(())
    }
    pub(crate) fn control(
        &self,
        arguments: &[u8],
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let mut r = Reader::new(arguments);
        let recipient = r.u64()?;
        let payload = r.blob()?;
        r.finish()?;
        ensure!(
            recipient == 1_000_001 && (5..=4096).contains(&payload.len()),
            "invalid game control recipient/size"
        );
        let size = u32::from_le_bytes(payload[..4].try_into()?) as usize;
        ensure!(size == payload.len() - 4, "game command length mismatch");
        let command = std::str::from_utf8(&payload[4..])?;
        ensure!(
            command.is_ascii()
                && !command
                    .bytes()
                    .any(|b| (b < 32 && !b.is_ascii_whitespace()) || b == 127),
            "invalid game command text"
        );
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        let mut response = TaskResult::success("local_game_control", vec![]);
        if command.starts_with("mmbook ") {
            let parts = command.split(' ').collect::<Vec<_>>();
            ensure!(parts.len() == 4, "invalid mmbook fields");
            let playlist_version = parts[1].parse::<u32>()?;
            let id: [u8; 8] = STANDARD
                .decode(parts[2])?
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid mmbook ID"))?;
            ensure!(matches!(parts[3], "0" | "1"), "unsupported booking kind");
            let admission = entries
                .get_mut(&id)
                .filter(|a| {
                    a.expires > now
                        && a.title == session.title_id
                        && a.owner == session.user_id
                        && a.seed == session.iv_seed
                })
                .ok_or_else(|| anyhow::anyhow!("booking does not belong to session"))?;
            admission.booked = true;
            admission.playlist_version = Some(playlist_version);
            let reply = b"mmbook accepted";
            let mut body = (reply.len() as u32).to_le_bytes().to_vec();
            body.extend_from_slice(reply);
            // BD_GLOBAL_INSTANT_MESSAGE=21 (target enum table 0xcd6d80).
            let mut push = Writer::default();
            push.u32(21);
            push.u64(1_000_001);
            push.string("Local Game Endpoint");
            push.blob(&body)?;
            response.label = "local_game_booked";
            response.pushes.push(push.0);
        } else if command.starts_with("mmstart ") {
            let admission = entries
                .values_mut()
                .find(|a| {
                    a.expires > now
                        && a.title == session.title_id
                        && a.owner == session.user_id
                        && a.seed == session.iv_seed
                        && a.booked
                })
                .ok_or_else(|| anyhow::anyhow!("game start requires booking"))?;
            if admission.story_channel.is_some() {
                let selected = story_capture::selection(
                    command,
                    self.story_capture_map
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("story capture policy missing"))?,
                )?;
                tracing::info!(map=%selected.map,mode="coop",difficulty=selected.difficulty,
                    "native story transport start accepted");
            }
            // Capture configuration for protocol recovery; never execute received console text.
            admission.start_command = Some(command.to_owned());
            response.label = "local_game_transport_start_requested";
            tracing::info!(
                bytes = command.len(),
                map = crate::content::maps::selection::from_mmstart(command)
                    .ok()
                    .flatten()
                    .map(|s| s.map),
                "native game start configuration received"
            );
        } else {
            return Ok(TaskResult::failure(
                "unsupported_game_control",
                task::TASK_FAILED,
            ));
        }
        Ok(response)
    }
    pub(crate) fn revoke_other_sessions(&self, session: &Session) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|_, a| {
                a.title != session.title_id
                    || a.owner != session.user_id
                    || a.seed == session.iv_seed
            });
        }
    }
    /// Only an authenticated lobby frame in continuous play may extend an
    /// existing live allocation. Never revive an expired or revoked entry.
    pub(crate) fn renew_session(&self, session: &Session, now: u32) -> Result<()> {
        ensure!(session.expires_at > now, "expired game lease session");
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        for entry in entries.values_mut().filter(|a| {
            a.expires > now
                && a.title == session.title_id
                && a.owner == session.user_id
                && a.seed == session.iv_seed
        }) {
            entry.expires = session.expires_at.min(now.saturating_add(930));
        }
        Ok(())
    }
    pub(crate) fn revoke(&self, session: &Session) {
        if let Ok(mut e) = self.entries.lock() {
            e.retain(|_, a| {
                !(a.title == session.title_id
                    && a.owner == session.user_id
                    && a.seed == session.iv_seed)
            });
        }
    }
    pub(crate) fn get(&self, id: &[u8; 8], now: u32) -> Option<Admission> {
        self.entries
            .lock()
            .ok()?
            .get(id)
            .filter(|a| a.expires > now && a.booked && a.start_command.is_some())
            .cloned()
    }
}

#[cfg(test)]
mod story_tests;
#[cfg(test)]
mod tests;
