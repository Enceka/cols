use super::*;

impl Connection {
    pub(crate) fn set_player_name(&mut self, name: Option<String>) {
        self.configured_name = name;
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
    pub(crate) fn with_resource_collection(mut self) -> Self {
        self.resource_collection = true;
        self
    }
    pub fn poll_profile_ready(&mut self) -> Option<&'static [u8]> {
        if self.profile_ready_sent
            || !self.connected
            || self.world.player.is_none()
            || self.gamestate_update != Some(6)
            || self
                .world
                .game_state
                .as_ref()
                .is_none_or(|s| s.profile_hash.is_none())
            || !self.hud_rules_sent
            || self.server_commands.acknowledged() < self.hud_rules_sequence
            || self
                .snapshot_first_sequence
                .is_none_or(|first| self.native_ack < first)
            || self.match_leave_sequence.is_some()
        {
            return None;
        }
        // Native0x4671b0 ->0x466790 ->0x62d340 dispatches this registered
        // connectionless notification.0xb49760 registers type1;0x5fa580
        // selects server profile persistence and makes the XP HUD ready.
        // It must use the authenticated DATA connection, not svc_serverCommand
        // (whose first-character dispatcher interprets 's' as dvar settings).
        self.profile_ready_sent = true;
        Some(b"\xff\xff\xff\xffserverWillWriteProfile\0")
    }
    pub fn decode_native(&self, payload: &[u8]) -> Result<(ClientHeader, Vec<u8>)> {
        let challenge = self
            .challenge
            .ok_or_else(|| anyhow::anyhow!("native message before challenge"))?;
        let header = ClientHeader::parse(payload)?;
        let command = self.server_commands.command_for(header.reliable_ack)?;
        Ok(game_wire::open_client(payload, challenge, command)?)
    }
    pub(super) fn native(&mut self, payload: &[u8]) -> Result<Option<Vec<u8>>> {
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
                codol_protocol::game::channel::usercmd::key(
                    header.message_ack,
                    0,
                    self.server_commands
                        .command_for(header.reliable_ack)
                        .expect("decoded slot"),
                )
            });
        let base = if key.is_some() && !self.resource_collection {
            self.input_base_for(header.message_ack)?
        } else {
            Default::default()
        };
        let received = if self.resource_collection {
            game_wire::client_controls_without_player_state(&clear)?
        } else {
            game_wire::client_message_with_base(&clear, key, &base)?
        };
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
                if let Ok(text) = std::str::from_utf8(&command) {
                    if text == "s" {
                        self.scoreboard_sent = false;
                    }
                    if text.starts_with("mr ") || text == "s" || text.starts_with("class ") {
                        tracing::info!(command=%text,"game menu request");
                    }
                }
                match crate::game::player::class_selection::from_client_command(&command, 1) {
                    Ok(Some(slot)) => {
                        self.requested_backpack = Some(slot);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error,"declined unsupported backpack menu request");
                    }
                }
                // Only explicitly implemented semantic requests are applied.
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
        let reply = if !self.native_fragments.is_empty() {
            None
        } else if self.native_sent == 0 {
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
            && let Some(state) = &self.world.game_state
        {
            let names = self
                .shared
                .as_ref()
                .map(|b| {
                    b.world
                        .lock()
                        .map_err(|_| anyhow::anyhow!("shared match poisoned"))?
                        .names(b.user)
                })
                .transpose()?;
            let body = state.body_roster(
                commands.sequence(),
                self.display_name.as_deref(),
                names.as_deref(),
            )?;
            let mut packets =
                game_wire::compact_server_packets(3, client_ack, challenge, client_command, &body)?;
            let reply = packets.remove(0);
            self.native_fragments.extend(packets);
            sending_gamestate = true;
            Some(reply)
        } else if self.resource_collection || client_ack > self.client_commands.sequence() {
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
        if !self.resource_collection
            && let Some(movement) = received.movement
        {
            self.predicted_position = Some((
                movement.predicted_time,
                movement.predicted_origin_bits.map(f32::from_bits),
            ));
            for input in movement.commands {
                if let Some(binding) = &self.shared {
                    self.input_count = self.input_count.saturating_add(1);
                    self.last_input = Some(input.clone());
                    binding
                        .world
                        .lock()
                        .map_err(|_| anyhow::anyhow!("shared match poisoned"))?
                        .submit(binding.user, input)?;
                } else {
                    self.queue_input(input);
                }
            }
            if self.shared.is_none() {
                self.consume_inputs(self.last_snapshot_time.max(1000));
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
            let name = validate_connect(connect, owner, build, challenge)?;
            self.display_name = Some(self.configured_name.clone().unwrap_or(name));
            self.owner_id = owner;
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
