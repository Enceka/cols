//! Application connection bootstrap after authenticated game DATA.
use anyhow::{Result, ensure};
use codol_protocol::game_wire::{self, ClientHeader, ReliableCommands, SERVER_EOF};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Connection {
    challenge: Option<u32>,
    connected: bool,
    stats: codol_protocol::game_stats::Upload,
    native_sent: u32,
    native_received: u32,
    native_ack: u32,
    server_commands: ReliableCommands,
    game_state: Option<Arc<crate::game_state::PreparedGameState>>,
    gamestate_sent: bool,
    gamestate_acknowledged: bool,
    gamestate_update: Option<u8>,
    client_commands: ReliableCommands,
    snapshot_count: u32,
    snapshot_next: Option<Instant>,
    snapshot_first_sequence: Option<u32>,
    input_count: u32,
    last_input: Option<codol_protocol::game_usercmd::UserCommand>,
    snapshot_input_bases: VecDeque<(u32, codol_protocol::game_usercmd::UserCommand)>,
}

impl Connection {
    pub fn with_game_state(state: Arc<crate::game_state::PreparedGameState>) -> Self {
        Self {
            game_state: Some(state),
            ..Self::default()
        }
    }

    pub fn stage(&self) -> &'static str {
        if self.gamestate_acknowledged {
            "gamestate_acknowledged"
        } else if self.gamestate_update == Some(6) {
            "gamestate_initialized"
        } else if self.gamestate_update.is_some() {
            "gamestate_parser_entered"
        } else if self.gamestate_sent {
            "gamestate_sent"
        } else if self.server_commands.acknowledged() == 1 {
            "reliable_probe_acknowledged"
        } else if self.stats.profile().is_some() {
            "stats_received"
        } else if self.connected {
            "connected"
        } else if self.challenge.is_some() {
            "challenged"
        } else {
            "unconnected"
        }
    }

    pub fn gamestate_progress(&self) -> Option<u8> {
        self.gamestate_update
    }

    /// An explicitly bounded stream at 20 Hz. The probe has no physics/input
    /// execution and is disabled unless explicitly selected by the operator.
    pub fn poll_snapshot(&mut self, now: Instant) -> Result<Option<Vec<u8>>> {
        if self.gamestate_update != Some(6)
            || self.snapshot_count >= self.game_state.as_ref().map_or(0, |s| s.snapshot_limit())
            || !self.game_state.as_ref().is_some_and(|s| s.snapshot_probe())
            || self.snapshot_next.is_some_and(|next| now < next)
        {
            return Ok(None);
        }
        let state = self.game_state.as_ref().expect("enabled state");
        let angles = if state.has_spawn() {
            self.view_angles()
        } else {
            [0; 3]
        };
        let server_time = 1000 + self.snapshot_count * 50;
        // Only view input is applied by this bootstrap. Advancing commandTime
        // with the snapshot clock suppresses local prediction (0x4522b0).
        let command_time = self
            .last_input
            .as_ref()
            .map_or(1000, |cmd| cmd.time)
            .min(server_time);
        let body = state.snapshot(
            server_time,
            command_time,
            self.snapshot_first_sequence
                .is_none_or(|first| self.native_ack < first),
            angles,
        )?;
        let ack = self.client_commands.sequence();
        let packet = game_wire::server_packet(
            self.native_sent + 1,
            ack,
            self.challenge
                .ok_or_else(|| anyhow::anyhow!("snapshot before challenge"))?,
            self.client_commands.command_for(ack)?,
            &body,
        )?;
        self.native_sent += 1;
        let mut base = state.input_base();
        // 0x5a18e0 copies pitch/yaw only; the default command's roll stays0.
        base.angles = [angles[0], angles[1], 0];
        self.snapshot_input_bases
            .push_back((self.native_sent, base));
        while self.snapshot_input_bases.len() > 256 {
            self.snapshot_input_bases.pop_front();
        }
        self.snapshot_first_sequence.get_or_insert(self.native_sent);
        self.snapshot_count += 1;
        self.snapshot_next = Some(now + Duration::from_millis(50));
        Ok(Some(packet))
    }
    pub fn input_progress(&self) -> (u32, Option<u32>) {
        (self.input_count, self.last_input.as_ref().map(|c| c.time))
    }
    pub fn view_angles(&self) -> [u16; 3] {
        self.last_input.as_ref().map_or([0; 3], |c| c.angles)
    }
    fn input_base_for(&self, ack: u32) -> Result<codol_protocol::game_usercmd::UserCommand> {
        self.snapshot_input_bases
            .iter()
            .rev()
            .find(|(sequence, _)| *sequence <= ack)
            .map(|(_, base)| base.clone())
            .ok_or_else(|| anyhow::anyhow!("no retained input base for snapshot ACK"))
    }

    pub fn decode_native(&self, payload: &[u8]) -> Result<(ClientHeader, Vec<u8>)> {
        let challenge = self
            .challenge
            .ok_or_else(|| anyhow::anyhow!("native message before challenge"))?;
        let header = ClientHeader::parse(payload)?;
        let command = self.server_commands.command_for(header.reliable_ack)?;
        Ok(game_wire::open_client(payload, challenge, command)?)
    }

    fn native(&mut self, payload: &[u8]) -> Result<Option<Vec<u8>>> {
        // Observed 2-byte association maintenance packet has no native header.
        if payload.len() < 4 {
            return Ok(None);
        }
        ensure!(
            self.connected && self.stats.profile().is_some(),
            "native message before stats completion"
        );
        let (header, clear) = self.decode_native(payload)?;
        ensure!(
            header.server_id == 0 || (self.gamestate_sent && header.server_id == 1),
            "unexpected server id"
        );
        ensure!(
            header.sequence > self.native_received,
            "native sequence replay"
        );
        ensure!(
            header.message_ack >= self.native_ack && header.message_ack <= self.native_sent,
            "native ACK outside sent range"
        );
        let key = self
            .snapshot_first_sequence
            .filter(|first| header.message_ack >= *first)
            .map(|_| {
                codol_protocol::game_usercmd::key(
                    header.message_ack,
                    0,
                    self.server_commands
                        .command_for(header.reliable_ack)
                        .expect("decoded slot"),
                )
            });
        let base = if key.is_some() {
            self.input_base_for(header.message_ack)?
        } else {
            Default::default()
        };
        let received = game_wire::client_message_with_base(&clear, key, &base)?;
        ensure!(
            self.gamestate_sent || received.commands.is_empty(),
            "client commands before gamestate"
        );
        let mut client_commands = self.client_commands.clone();
        for (sequence, command) in received.commands {
            if sequence <= client_commands.sequence() {
                ensure!(
                    client_commands.command_for(sequence)? == command,
                    "changed client command retry"
                );
            } else {
                ensure!(
                    sequence == client_commands.sequence() + 1,
                    "client command sequence gap"
                );
                // Receipt only: no console/gameplay execution is implied by ACK.
                client_commands.push(&command)?;
                client_commands.mark_sent(sequence)?;
                client_commands.acknowledge(sequence)?;
            }
        }
        let mut commands = self.server_commands.clone();
        commands.acknowledge(header.reliable_ack)?;
        let challenge = self.challenge.expect("decoded challenge");
        let client_ack = client_commands.sequence();
        let client_command = client_commands.command_for(client_ack)?;
        let mut sending_gamestate = false;
        let reply = if self.native_sent == 0 {
            Some(game_wire::server_packet(
                1,
                client_ack,
                challenge,
                client_command,
                &[SERVER_EOF],
            )?)
        } else if self.native_sent == 1 && header.message_ack == 1 {
            // Empty server command exercises the receiver and command ACK with
            // no configstring mutation. Client command ACK remains zero.
            let sequence = commands.push(b"")?;
            let mut body = vec![1];
            body.extend_from_slice(&sequence.to_le_bytes());
            body.extend_from_slice(&[0, SERVER_EOF]);
            let reply = game_wire::server_packet(2, client_ack, challenge, client_command, &body)?;
            commands.mark_sent(sequence)?;
            Some(reply)
        } else if self.native_sent == 2
            && header.message_ack == 2
            && header.reliable_ack == 1
            && let Some(state) = &self.game_state
        {
            let body = state.body(commands.sequence())?;
            let reply = game_wire::server_packet(3, client_ack, challenge, client_command, &body)?;
            sending_gamestate = true;
            Some(reply)
        } else if client_ack > self.client_commands.sequence() {
            Some(game_wire::server_packet(
                self.native_sent + 1,
                client_ack,
                challenge,
                client_command,
                &[SERVER_EOF],
            )?)
        } else {
            None
        };
        self.server_commands = commands;
        self.client_commands = client_commands;
        if let Some(movement) = received.movement {
            for input in movement.commands {
                if self
                    .last_input
                    .as_ref()
                    .is_none_or(|last| input.time > last.time)
                {
                    self.input_count += 1;
                    self.last_input = Some(input);
                }
            }
        }
        self.gamestate_sent |= sending_gamestate;
        self.gamestate_acknowledged |=
            self.gamestate_sent && header.message_ack >= 3 && header.server_id == 1;
        self.native_received = header.sequence;
        self.native_ack = header.message_ack;
        if reply.is_some() {
            self.native_sent += 1;
        }
        Ok(reply)
    }

    pub fn handle(&mut self, payload: &[u8], owner: u64, build: i32) -> Result<Option<Vec<u8>>> {
        let Some(command) = payload.strip_prefix(&[255; 4]) else {
            return self.native(payload);
        };
        if let Some(update) = command.strip_prefix(b"gamestateupdate ") {
            ensure!(self.gamestate_sent, "gamestate update before send");
            let value = update.strip_suffix(&[0]).unwrap_or(update);
            ensure!(
                matches!(value, b"1" | b"2" | b"3" | b"4" | b"5" | b"6"),
                "invalid gamestate update"
            );
            self.gamestate_update = Some(self.gamestate_update.unwrap_or(0).max(value[0] - b'0'));
            return Ok(None);
        }
        if let Some(chunk) = command.strip_prefix(b"stats\0") {
            ensure!(self.connected, "stats before connect");
            let missing = self.stats.accept(chunk)?;
            let mut reply = vec![255; 4];
            reply.extend_from_slice(format!("statResponse {missing}\0").as_bytes());
            return Ok(Some(reply));
        }
        if command.strip_suffix(&[0]).unwrap_or(command) == b"heartbeat" {
            ensure!(
                self.stats.profile().is_some(),
                "heartbeat before stats completion"
            );
            // Target 0x466c30 accepts this only in state 6 and advances when
            // the local map-loading worker has completed (0x4640a0).
            return Ok(Some(b"\xff\xff\xff\xffheartbeatresponse\0".to_vec()));
        }
        if let Some(connect) = command.strip_prefix(b"connect ") {
            let challenge = self
                .challenge
                .ok_or_else(|| anyhow::anyhow!("connect before challenge"))?;
            validate_connect(connect, owner, build, challenge)?;
            self.connected = true;
            // 0x466b24..0x466c22 initializes netchan, enters state 5 and starts
            // uploading stats. This target does not consume an info-string here.
            return Ok(Some(b"\xff\xff\xff\xffconnectResponse\0".to_vec()));
        }
        if !command.starts_with(b"getchallenge ") {
            return Ok(None);
        }
        ensure!(command.len() <= 128, "game challenge request too long");
        let command = command.strip_suffix(&[0]).unwrap_or(command);
        let command = std::str::from_utf8(command)?;
        let fields = command.split(' ').collect::<Vec<_>>();
        ensure!(
            fields.len() == 3 && fields[1] == "0",
            "unsupported challenge request"
        );
        let uid = fields[2]
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(|| anyhow::anyhow!("missing challenge identity"))?;
        ensure!(
            uid.len() == 16 && uid.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid challenge identity"
        );
        ensure!(
            u64::from_str_radix(uid, 16)? == owner,
            "challenge identity differs from admission"
        );
        let challenge = if let Some(value) = self.challenge {
            value
        } else {
            let mut random = [0; 4];
            openssl::rand::rand_bytes(&mut random)?;
            let value = (u32::from_le_bytes(random) & i32::MAX as u32).max(1);
            self.challenge = Some(value);
            value
        };
        // Target 0x466a55 reads argv(1) as an integer into connection +0x138,
        // then changes state from challenging (3) to connecting (4).
        let mut reply = vec![255; 4];
        reply.extend_from_slice(format!("challengeResponse {challenge}\0").as_bytes());
        Ok(Some(reply))
    }
}

fn identity(bytes: &[u8]) -> Result<u64> {
    ensure!(
        bytes.len() == 16 && bytes.iter().all(u8::is_ascii_hexdigit),
        "invalid connect identity"
    );
    Ok(u64::from_str_radix(std::str::from_utf8(bytes)?, 16)?)
}

fn validate_connect(body: &[u8], owner: u64, build: i32, challenge: u32) -> Result<()> {
    ensure!(body.len() <= 1024, "connect request too long");
    let body = body.strip_suffix(&[0]).unwrap_or(body);
    ensure!(
        body.len() >= 20 && body[16] == b' ',
        "invalid connect framing"
    );
    ensure!(
        identity(&body[..16])? == owner,
        "connect identity differs from admission"
    );
    let info = body[17..]
        .strip_prefix(b"\"")
        .and_then(|s| s.strip_suffix(b"\""))
        .ok_or_else(|| anyhow::anyhow!("connect userinfo must be quoted"))?;
    ensure!(
        !info.iter().any(|b| *b < 32 || *b == 127 || *b == b'"'),
        "invalid connect userinfo character"
    );
    let info = info
        .strip_prefix(b"\\")
        .ok_or_else(|| anyhow::anyhow!("invalid userinfo prefix"))?;
    let mut fields = std::collections::BTreeMap::<&[u8], &[u8]>::new();
    let mut parts = info.split(|b| *b == b'\\');
    while let Some(key) = parts.next() {
        let value = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("unpaired userinfo field"))?;
        ensure!(
            !key.is_empty()
                && key.len() <= 64
                && key.is_ascii()
                && value.len() <= 512
                && fields.len() < 64,
            "userinfo field limit"
        );
        ensure!(
            fields.insert(key, value).is_none(),
            "duplicate userinfo field"
        );
    }
    let get = |key: &[u8]| -> Result<&[u8]> {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("missing required connect field"))
    };
    ensure!(
        identity(get(b"xuid")?)? == owner,
        "userinfo identity differs from admission"
    );
    ensure!(
        std::str::from_utf8(get(b"challenge")?)?.parse::<u32>()? == challenge,
        "connect challenge mismatch"
    );
    ensure!(
        std::str::from_utf8(get(b"2490")?)?.parse::<i32>()? == build,
        "connect build mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_baseline_follows_acknowledged_snapshot_not_latest_view() {
        let mut c = Connection::default();
        let first = codol_protocol::game_usercmd::UserCommand {
            angles: [100, 200, 0],
            weapons: [352, 0, 0],
            ..Default::default()
        };
        let second = codol_protocol::game_usercmd::UserCommand {
            angles: [300, 400, 0],
            ..first.clone()
        };
        c.snapshot_input_bases.push_back((4, first.clone()));
        c.snapshot_input_bases.push_back((6, second.clone()));
        assert!(c.input_base_for(3).is_err());
        assert_eq!(c.input_base_for(4).unwrap(), first);
        assert_eq!(c.input_base_for(5).unwrap(), first); // reliable-only message
        assert_eq!(c.input_base_for(6).unwrap(), second);
    }
    #[test]
    fn snapshot_probe_requires_opt_in_initialization_and_has_rate_and_count_limits() {
        let now = Instant::now();
        let mut disabled = Connection::with_game_state(Arc::new(crate::game_state::synthetic()));
        disabled.gamestate_update = Some(6);
        assert!(disabled.poll_snapshot(now).unwrap().is_none());
        let mut c = Connection::with_game_state(Arc::new(
            crate::game_state::synthetic()
                .with_snapshot_frames(60)
                .unwrap(),
        ));
        c.challenge = Some(0x75);
        c.native_sent = 3;
        assert!(c.poll_snapshot(now).unwrap().is_none());
        c.gamestate_update = Some(5);
        assert!(c.poll_snapshot(now).unwrap().is_none());
        c.gamestate_update = Some(6);
        for i in 0..60u32 {
            if i == 1 {
                c.native_ack = 4;
            }
            let at = now + Duration::from_millis(u64::from(i) * 50);
            let mut packet = c.poll_snapshot(at).unwrap().unwrap();
            assert_eq!(&packet[..4], &(i + 4).to_le_bytes());
            game_wire::xor_stream(&mut packet[8..], 0x75 ^ (i + 4) as u8, b"");
            assert_eq!(packet[14], if i == 0 { 8 } else { 0 });
            assert_eq!(&packet[9..13], &(1000 + i * 50).to_le_bytes());
            // The frozen captured layout places PS.commandTime at body+9.
            // Sending more frames without input must not acknowledge input
            // that the client has not generated yet.
            assert_eq!(&packet[17..21], &1000u32.to_le_bytes());
            assert!(
                c.poll_snapshot(at + Duration::from_millis(49))
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            c.poll_snapshot(now + Duration::from_secs(60))
                .unwrap()
                .is_none()
        );
        assert_eq!(c.native_sent, 63);
        assert_eq!(c.native_received, 0); // sending must never invent client ACKs
    }
    #[test]
    fn stats_and_heartbeat_require_prior_stages_and_all_fragments() {
        let mut connection = Connection::default();
        let heartbeat = b"\xff\xff\xff\xffheartbeat\0";
        assert!(connection.handle(heartbeat, 42, 7).is_err());
        let raw = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (index, data) in raw
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut packet = b"\xff\xff\xff\xffstats\0".to_vec();
            packet.extend_from_slice(&[index as u8, 0, 0]);
            packet.extend_from_slice(data);
            if index == 0 {
                assert!(connection.handle(&packet, 42, 7).is_err());
                connection.connected = true;
            }
            let reply = connection.handle(&packet, 42, 7).unwrap().unwrap();
            if index < 26 {
                assert_ne!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
                assert!(connection.handle(heartbeat, 42, 7).is_err());
            } else {
                assert_eq!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
            }
        }
        assert_eq!(connection.stage(), "stats_received");
        assert_eq!(
            connection.handle(heartbeat, 42, 7).unwrap().unwrap(),
            b"\xff\xff\xff\xffheartbeatresponse\0"
        );
    }
    #[test]
    fn challenge_requires_admitted_identity_and_retries_reuse_nonce() {
        let mut connection = Connection::default();
        let request = b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\"\0";
        assert!(connection.handle(request, 1, 7).is_err());
        assert!(connection.challenge.is_none());
        let response = connection
            .handle(request, u64::MAX - 1, 7)
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"\xff\xff\xff\xffchallengeResponse "));
        assert_eq!(
            connection
                .handle(request, u64::MAX - 1, 7)
                .unwrap()
                .unwrap(),
            response
        );
        assert!(
            connection
                .handle(
                    b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\" trailing",
                    u64::MAX - 1,
                    7
                )
                .is_err()
        );
        assert!(
            connection
                .handle(&[0, 0], u64::MAX - 1, 7)
                .unwrap()
                .is_none()
        );
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffarbitrary", u64::MAX - 1, 7)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn connect_binds_both_identities_build_and_nonce_without_duplicate_fields() {
        let mut connection = Connection::default();
        let valid = b"\xff\xff\xff\xffconnect fffffffffffffffe \"\\name\\Lab player\\xuid\\fffffffffffffffe\\challenge\\12345\\2490\\-688906341\"";
        assert!(connection.handle(valid, u64::MAX - 1, -688906341).is_err());
        connection.challenge = Some(12345);
        for (old, new) in [
            ("fffffffffffffffe", "ffffffffffffffff"),
            ("12345", "12346"),
            ("-688906341", "0"),
            ("\\name\\", "\\xuid\\"),
        ] {
            let text = std::str::from_utf8(&valid[4..]).unwrap().replace(old, new);
            let mut damaged = vec![255; 4];
            damaged.extend_from_slice(text.as_bytes());
            assert!(
                connection
                    .handle(&damaged, u64::MAX - 1, -688906341)
                    .is_err()
            );
            assert!(!connection.connected);
        }
        for _ in 0..2 {
            assert_eq!(
                connection
                    .handle(valid, u64::MAX - 1, -688906341)
                    .unwrap()
                    .unwrap(),
                b"\xff\xff\xff\xffconnectResponse\0"
            );
        }
        assert_eq!(connection.stage(), "connected");
        let mut extra = valid.to_vec();
        extra.extend_from_slice(b" garbage");
        assert!(connection.handle(&extra, u64::MAX - 1, -688906341).is_err());
    }

    fn eof(sequence: u32, message_ack: u32, reliable_ack: u32) -> Vec<u8> {
        let mut wire = sequence.to_le_bytes().to_vec();
        wire.push(0);
        wire.extend_from_slice(&message_ack.to_le_bytes());
        wire.extend_from_slice(&reliable_ack.to_le_bytes());
        wire.push(3 ^ 0x75 ^ message_ack as u8);
        wire
    }

    #[test]
    fn probes_keep_client_ack_zero_terminate_stream_and_require_valid_progress() {
        let mut connection = Connection {
            challenge: Some(0x75),
            connected: true,
            ..Default::default()
        };
        assert!(connection.handle(&eof(1, 0, 0), 1, 7).is_err());
        let profile = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (i, chunk) in profile
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut part = vec![i as u8, 0, 0];
            part.extend_from_slice(chunk);
            connection.stats.accept(&part).unwrap();
        }
        assert!(connection.handle(&eof(1, 1, 0), 1, 7).is_err());
        let first = connection.handle(&eof(1, 0, 0), 1, 7).unwrap().unwrap();
        assert_eq!(&first[..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(first[8], 6 ^ 0x75 ^ 1);
        assert!(connection.handle(&eof(1, 0, 0), 1, 7).is_err());
        assert!(connection.handle(&eof(2, 1, 1), 1, 7).is_err());
        assert_eq!(connection.native_received, 1); // rejection must not consume sequence
        let mut second = connection.handle(&eof(2, 1, 0), 1, 7).unwrap().unwrap();
        assert_eq!(&second[..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
        game_wire::xor_stream(&mut second[8..], 0x75 ^ 2, b"");
        assert_eq!(&second[8..], &[1, 1, 0, 0, 0, 0, 6]);
        assert!(connection.handle(&eof(3, 3, 1), 1, 7).is_err());
        assert!(connection.handle(&eof(3, 2, 1), 1, 7).unwrap().is_none());
        assert_eq!(connection.stage(), "reliable_probe_acknowledged");
        assert!(connection.handle(&eof(4, 1, 0), 1, 7).is_err());
    }

    #[test]
    fn gamestate_requires_probe_ack_and_client_commands_commit_atomically() {
        let mut connection = Connection::with_game_state(Arc::new(crate::game_state::synthetic()));
        connection.challenge = Some(0x75);
        connection.connected = true;
        let profile = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (i, chunk) in profile
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut part = vec![i as u8, 0, 0];
            part.extend_from_slice(chunk);
            connection.stats.accept(&part).unwrap();
        }
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffgamestateupdate 1\0", 1, 7)
                .is_err()
        );
        connection.handle(&eof(1, 0, 0), 1, 7).unwrap();
        connection.handle(&eof(2, 1, 0), 1, 7).unwrap();
        assert!(connection.handle(&eof(3, 2, 0), 1, 7).unwrap().is_none());
        let mut packet = connection.handle(&eof(4, 2, 1), 1, 7).unwrap().unwrap();
        assert_eq!(packet[..8], [3, 0, 0, 0, 0, 0, 0, 0]);
        game_wire::xor_stream(&mut packet[8..], 0x75 ^ 3, b"");
        assert_eq!(packet[8], 3);
        assert_eq!(packet.last(), Some(&6));
        assert_eq!(connection.stage(), "gamestate_sent");
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 1\0", 1, 7)
            .unwrap();
        assert_eq!(connection.stage(), "gamestate_parser_entered");
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 6\0", 1, 7)
            .unwrap();
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 3\0", 1, 7)
            .unwrap();
        assert_eq!(connection.stage(), "gamestate_initialized");
        assert_eq!(connection.gamestate_progress(), Some(6));
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffgamestateupdate 7\0", 1, 7)
                .is_err()
        );
        // One reliable command, then EOF in the same pool byte (2 | 3<<3).
        let mut wire = eof(5, 3, 1);
        wire[4] = 1;
        wire.truncate(13);
        let mut body = vec![0x1a, 1, 0, 0, 0, b'a', 0];
        game_wire::xor_stream(&mut body, 0x75 ^ 3 ^ 1, b"");
        wire.extend_from_slice(&body);
        let mut broken = wire.clone();
        broken.pop();
        assert!(connection.handle(&broken, 1, 7).is_err());
        assert_eq!(connection.client_commands.sequence(), 0);
        let mut response = connection.handle(&wire, 1, 7).unwrap().unwrap();
        assert_eq!(response[..8], [4, 0, 0, 0, 1, 0, 0, 0]);
        game_wire::xor_stream(&mut response[8..], 0x75 ^ 4, b"a");
        assert_eq!(response[8..], [6]);
        assert_eq!(connection.stage(), "gamestate_acknowledged");
        wire[..4].copy_from_slice(&6u32.to_le_bytes());
        assert!(connection.handle(&wire, 1, 7).unwrap().is_none());
        assert_eq!(connection.client_commands.sequence(), 1);
    }
}
