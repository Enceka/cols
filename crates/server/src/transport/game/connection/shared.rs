use super::*;
use crate::game::session::shared::SharedMatch;
use std::sync::Mutex;

pub(super) struct Binding {
    pub(super) world: Arc<Mutex<SharedMatch>>,
    pub(super) user: u64,
    revision: u64,
    history: VecDeque<(u32, crate::game::session::shared::View, bool)>,
}
impl Connection {
    pub(crate) fn with_shared(world: Arc<Mutex<SharedMatch>>, user: u64) -> Result<Self> {
        let state = {
            let mut guard = world
                .lock()
                .map_err(|_| anyhow::anyhow!("shared match poisoned"))?;
            guard.attach(user)?;
            guard.state(user)?
        };
        Ok(Self {
            world: crate::game::session::local_world::LocalWorld {
                game_state: Some(state.clone()),
                player: state.player(),
                motion: state.motion(),
                ..Default::default()
            },
            view_offset: state.spawn_angles(),
            known_weapons: state
                .weapon_configstrings()
                .into_iter()
                .map(|(i, n)| (n, i - 5443))
                .collect(),
            next_weapon: state.next_weapon_index(),
            shared: Some(Binding {
                world,
                user,
                revision: 0,
                history: VecDeque::new(),
            }),
            ..Default::default()
        })
    }
    pub(crate) fn shared_user(&self) -> Option<u64> {
        self.shared.as_ref().map(|b| b.user)
    }
    pub(crate) fn detach_shared(&self) {
        if let Some(binding) = &self.shared
            && let Ok(mut world) = binding.world.lock()
        {
            world.detach(binding.user);
        }
    }
    pub(super) fn poll_shared_snapshot(&mut self, wall: Instant) -> Result<Option<Vec<u8>>> {
        if self.gamestate_update != Some(6)
            || self.snapshot_next.is_some_and(|next| wall < next)
            || self
                .match_leave_sequence
                .is_some_and(|seq| self.server_commands.acknowledged() >= seq)
        {
            return Ok(None);
        }
        let binding = self.shared.as_mut().unwrap();
        let mut world = binding
            .world
            .lock()
            .map_err(|_| anyhow::anyhow!("shared match poisoned"))?;
        if self
            .snapshot_first_sequence
            .is_some_and(|seq| self.native_ack >= seq)
        {
            world.ready(binding.user)?;
        }
        let view = world.view(binding.user)?;
        let names = world.names(binding.user)?;
        let previous = binding
            .history
            .iter()
            .rev()
            .find(|(sequence, _, _)| {
                *sequence <= self.native_ack && self.native_sent + 1 - sequence < 24
            })
            .cloned();
        let feedback_sounds = world.take_feedback_sounds(binding.user)?;
        drop(world);
        self.last_snapshot_time = view.time;
        if binding.revision != view.revision || !self.scoreboard_sent {
            for payload in
                codol_protocol::game::snapshot::scoreboard::binary_commands(&view.scoreboard)?
            {
                self.server_commands.push_binary(&payload)?;
            }
            binding.revision = view.revision;
            self.scoreboard_sent = true;
        }
        if !self.hud_rules_sent {
            self.server_commands.push(format!("s ui_scorelimit \"{}\" scr_gameended \"0\" ui_match_complete \"0\" ui_eog_show \"0\"", view.scoreboard.score_limit).as_bytes())?;
            self.hud_rules_sent = true;
            self.hud_rules_sequence = self.server_commands.sequence();
        }
        let state = self.world.game_state.as_ref().unwrap();
        if let Some(resources) = state.feedback_resources() {
            for _ in 0..feedback_sounds {
                self.server_commands
                    .push(resources.sound_command().as_bytes())?;
            }
        }
        if let Some(ended) = view.ended {
            if self.match_summary_sequence.is_none() {
                let teams = std::iter::once(view.local.team)
                    .chain(view.players.iter().map(|p| p.team))
                    .collect::<Vec<_>>();
                for payload in
                    codol_protocol::game::snapshot::scoreboard::summary_human_identity_commands(
                        &names, &teams,
                    )?
                {
                    self.server_commands.push_binary(&payload)?;
                }
                self.server_commands.push_binary(
                    &codol_protocol::game::snapshot::scoreboard::summary_command(
                        view.winner,
                        (ended - 1000) / 1000,
                        state.summary_map_id()?,
                        1,
                    )?,
                )?;
                self.match_summary_sequence = Some(self.server_commands.sequence());
            }
            if self.match_ui_sequence.is_none()
                && self.server_commands.acknowledged() >= self.match_summary_sequence.unwrap()
            {
                self.server_commands
                    .push(b"s scr_gameended \"1\" ui_match_complete \"1\" ui_eog_show \"1\"")?;
                self.match_ui_sequence = Some(self.server_commands.sequence());
            }
            if self.match_summary_opened.is_none()
                && self
                    .match_ui_sequence
                    .is_some_and(|seq| self.server_commands.acknowledged() >= seq)
            {
                self.match_summary_opened = Some(view.time);
            }
            if self
                .match_summary_opened
                .is_some_and(|start| view.time.saturating_sub(start) >= 12000)
                && self.match_leave_sequence.is_none()
            {
                self.match_leave_sequence = Some(self.server_commands.push(b"t EXE_MATCHENDED")?);
            }
        }
        let delta_distance = previous.as_ref().map_or(0, |(sequence, _, _)| {
            (self.native_sent + 1 - sequence) as u8
        });
        let snapshot = state.human_snapshot(
            &view,
            self.view_offset,
            previous.as_ref().map(|(_, v, _)| v),
            delta_distance,
            previous.as_ref().is_some_and(|(_, _, held)| *held),
        )?;
        let (mut body, mut sent) = self
            .server_commands
            .encode_frames_bounded(game_wire::FRAGMENT_SIZE.saturating_sub(snapshot.len() + 5))?;
        let included = !(body.is_empty()
            && self.server_commands.sequence() > self.server_commands.acknowledged()
            && self.snapshot_first_sequence.is_some());
        if included {
            body.extend_from_slice(&snapshot);
        } else {
            (body, sent) = self
                .server_commands
                .encode_frames_bounded(game_wire::FRAGMENT_SIZE - 6)?;
            ensure!(
                !body.is_empty(),
                "shared reliable record exceeds message budget"
            );
            body.push(SERVER_EOF);
        }
        let ack = self.client_commands.sequence();
        let mut packets = game_wire::compact_server_packets(
            self.native_sent + 1,
            ack,
            self.challenge
                .ok_or_else(|| anyhow::anyhow!("shared snapshot before challenge"))?,
            self.client_commands.command_for(ack)?,
            &body,
        )?;
        let packet = packets.remove(0);
        self.native_fragments.extend(packets);
        self.native_sent += 1;
        self.server_commands.mark_sent(sent)?;
        if !included {
            self.snapshot_next = Some(wall + Duration::from_millis(50));
            return Ok(Some(packet));
        }
        let history = &mut self.shared.as_mut().unwrap().history;
        history.push_back((self.native_sent, view.clone(), previous.is_some()));
        while history.len() > 32 {
            history.pop_front();
        }
        let mut base = state.input_base();
        base.weapons = view.equipment.input_weapons();
        base.angles = [
            view.angles[0].wrapping_sub(self.view_offset[0]),
            view.angles[1].wrapping_sub(self.view_offset[1]),
            0,
        ];
        if view.equipment.aim.is_some_and(|a| a.fraction != 0.0) {
            base.buttons |= 0x800;
        }
        self.remember_input_base(self.native_sent, base);
        self.snapshot_first_sequence.get_or_insert(self.native_sent);
        self.snapshot_count += 1;
        self.snapshot_next = Some(wall + Duration::from_millis(50));
        Ok(Some(packet))
    }
}
