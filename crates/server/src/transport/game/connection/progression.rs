//! Bridge a completed local world to one persistent reward transaction.
use super::*;
use crate::contracts::progression::{MatchAward, Policy};
impl Connection {
    pub(crate) fn has_tdm_world(&self) -> bool {
        self.world.tdm.is_some()
    }
    pub(crate) fn with_progression(mut self, policy: Policy, owned: &[u32]) -> Result<Self> {
        ensure!(
            policy.valid() && self.world.tdm.is_some() && !self.resource_collection,
            "progression requires an actual local TDM world"
        );
        let mut id = [0; 16];
        openssl::rand::rand_bytes(&mut id)?;
        self.match_progress = Some(crate::game::progression::MatchProgress::new(
            id.iter().map(|v| format!("{v:02x}")).collect(),
            policy,
            owned,
        ));
        Ok(self)
    }
    pub(crate) fn pending_match_award(&self) -> Option<MatchAward> {
        if self.progression_committed
            || !self.world.tdm.as_ref().is_some_and(|m| m.finished())
            || self
                .progression_retry_at
                .is_some_and(|next| Instant::now() < next)
        {
            return None;
        }
        self.match_progress.as_ref().and_then(|p| p.final_award())
    }
    pub(crate) fn progression_commit_failed(&mut self) {
        self.progression_retry_at = Some(Instant::now() + Duration::from_secs(1));
    }
    pub(crate) fn progression_resolved(&self) -> bool {
        self.progression_committed
            || self
                .match_progress
                .as_ref()
                .is_none_or(|p| p.final_award().is_none())
    }
    pub(crate) fn progression_committed(
        &mut self,
        receipt: &crate::backend::progression::Receipt,
        private: &[u8],
    ) -> Result<()> {
        if self.progression_committed {
            return Ok(());
        }
        ensure!(
            self.match_progress
                .as_ref()
                .and_then(|p| p.final_award())
                .is_some_and(|award| award.match_id == receipt.match_id),
            "receipt belongs to another local match"
        );
        let mut commands = self.server_commands.clone();
        commands.push_binary(
            &codol_protocol::game::snapshot::progression::private_progress(private, self.owner_id)?,
        )?;
        for weapon in &receipt.weapons {
            commands.push(&codol_protocol::game::snapshot::progression::weapon_xp(
                weapon.item_id,
                weapon.progress.after.total_xp,
            )?)?;
        }
        if !receipt.player.levels_gained.is_empty() {
            commands.push(&codol_protocol::game::snapshot::progression::promotion(
                receipt.player.after.level,
            )?)?;
            // AAR checks this measured native notification separately from
            // private rank and the live splash. One highest-rank promotion is
            // presented for a batch, while the receipt retains every level.
            commands.push(b"s ui_promotion \"1\"")?;
        }
        if let Some(state) = &mut self.world.game_state {
            let rank = codol_protocol::lobby::profile::player_rank(private, self.owner_id)?;
            Arc::make_mut(state).set_card_rank(rank);
            self.world_history.clear();
        }
        self.progression_ack = Some((
            commands.sequence(),
            receipt.match_id.clone(),
            receipt.revision,
        ));
        self.server_commands = commands;
        self.progression_committed = true;
        self.scoreboard_sent = false;
        self.progression_retry_at = None;
        tracing::info!(match_id=%receipt.match_id,revision=receipt.revision,
            player_xp=receipt.player.credited_xp,level=receipt.player.after.level, "local TDM progression committed");
        Ok(())
    }
    pub(crate) fn progression_delivery_ack(&self) -> Option<(String, u64)> {
        self.progression_ack
            .as_ref()
            .filter(|(sequence, _, _)| self.server_commands.acknowledged() >= *sequence)
            .map(|(_, id, revision)| (id.clone(), *revision))
    }
    pub(crate) fn progression_delivery_confirmed(&mut self) {
        self.progression_ack = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        modes::tdm::{MatchConfig, PlayerHit, TdmMatch},
        world::collision::Collision,
    };
    use codol_protocol::game::snapshot::enemy::EnemySnapshot;
    #[tokio::test]
    #[ignore = "uses locally captured Afghan collision/arsenal and isolated progression account fixtures"]
    async fn captured_afghan_shooting_commits_once_and_reloads_progress_through_business_services()
    {
        use crate::{app::services::LocalServices, contracts::account::AccountId};
        use codol_protocol::game::channel::usercmd::UserCommand;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research-output/progression-gameplay-20261014");
        let identity: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("identity.json")).unwrap()).unwrap();
        let account = AccountId {
            title_id: 1002,
            user_id: identity["user_id"].as_str().unwrap().parse().unwrap(),
        };
        let temp = tempfile::tempdir().unwrap();
        let directory = temp
            .path()
            .join(format!("profiles/1002/{}", account.user_id));
        std::fs::create_dir_all(&directory).unwrap();
        for name in ["public.bin", "private.bin", "inventory.json", "wallet.json"] {
            std::fs::copy(
                root.join(format!(
                    "service-data/profiles/1002/{}/{name}",
                    account.user_id
                )),
                directory.join(name),
            )
            .unwrap();
        }
        for name in ["progression-rules.json", "progression-policy.json"] {
            std::fs::copy(root.join("service-data").join(name), temp.path().join(name)).unwrap();
        }
        let services = LocalServices::open(temp.path().into()).await.unwrap();
        let seed = MatchAward {
            match_id: "11111111111111111111111111111111".into(),
            player_xp: 0,
            weapons: vec![crate::contracts::progression::WeaponXp {
                item_id: 10001,
                xp: 490,
            }],
        };
        services
            .commit_match_progression(account, seed)
            .await
            .unwrap();
        let original = services
            .game_profile(account.title_id, account.user_id)
            .await
            .unwrap();
        let template = crate::game::session::prepared::PreparedGameState::load(
            &root.join("afghan-state.json"),
        )
        .unwrap();
        let state = Arc::new(
            template
                .for_initial_backpack(&original.public)
                .unwrap()
                .for_profile(&original.public, &original.owned)
                .unwrap(),
        );
        let mut connection = Connection::with_game_state(state.clone())
            .with_progression(
                services.progression_policy().await.unwrap().unwrap(),
                &original.owned,
            )
            .unwrap();
        connection.owner_id = account.user_id;
        let local = state.local_player_snapshot(true).unwrap();
        let collision = state.collision.as_ref().unwrap();
        let enemies = connection.world.tdm.as_ref().unwrap().snapshots();
        let eye = [local.origin[0], local.origin[1], local.origin[2] + 60.];
        let target = enemies
            .iter()
            .filter(|actor| actor.team != local.team)
            .min_by(|a, b| {
                let distance = |actor: &&EnemySnapshot| {
                    (actor.origin[0] - eye[0]).hypot(actor.origin[1] - eye[1])
                };
                distance(a).total_cmp(&distance(b))
            })
            .unwrap();
        let delta = [
            target.origin[0] - eye[0],
            target.origin[1] - eye[1],
            target.origin[2] + 35. - eye[2],
        ];
        let angle = |v: f32| ((v.rem_euclid(360.) * 65536. / 360.).round() as u32 & 65535) as u16;
        let angles = [
            angle(-delta[2].atan2(delta[0].hypot(delta[1])).to_degrees()),
            angle(delta[1].atan2(delta[0]).to_degrees()),
            0,
        ];
        let weapon = connection.world.player.as_ref().unwrap().equipment().weapon;
        assert_eq!(state.progression_item(weapon), Some(10001));
        // Follow the prepared map's real opening clock before firing; mode
        // flow may now include a preparation interval even in older captures.
        let fire_start = (1000..=31_000)
            .step_by(100)
            .find(|&time| {
                let tdm = connection.world.tdm.as_mut().unwrap();
                tdm.advance_clock(time);
                !tdm.preparing()
            })
            .expect("captured match must leave preparation")
            .max(3000);
        let mut fired = 0;
        for time in (fire_start..=fire_start + 2000).step_by(100) {
            connection.world.tdm.as_mut().unwrap().advance_clock(time);
            connection
                .world
                .player
                .as_mut()
                .unwrap()
                .apply(&UserCommand {
                    time,
                    buttons: 0x801,
                    angles,
                    weapons: [weapon, 0, 0],
                    ..Default::default()
                });
            let shots = connection.world.player.as_mut().unwrap().take_shots();
            for shot in shots {
                fired += 1;
                connection
                    .world
                    .tdm
                    .as_mut()
                    .unwrap()
                    .player_fire(shot, eye, local, collision, None);
            }
            connection.drain_match_kills(time);
            if connection.world.tdm.as_ref().unwrap().finished() {
                break;
            }
        }
        assert!(fired >= 3, "must fire actual captured weapon shots");
        let award = connection
            .pending_match_award()
            .expect("actual shots must produce a completed enemy kill");
        assert_eq!(award.player_xp, 100);
        assert_eq!(
            award.weapons,
            [crate::contracts::progression::WeaponXp {
                item_id: 10001,
                xp: 100
            }]
        );
        let committed = services
            .commit_match_progression(account, award.clone())
            .await
            .unwrap();
        assert_eq!(committed.receipt.player.after.level, 2);
        assert_eq!(committed.receipt.player.after.xp_within_level, 0);
        assert_eq!(committed.receipt.weapons[0].progress.after.level, 2);
        assert_eq!(committed.receipt.weapons[0].progress.after.total_xp, 590);
        let private = services
            .private_game_profile(account)
            .await
            .unwrap()
            .unwrap();
        connection
            .progression_committed(&committed.receipt, &private)
            .unwrap();
        let frames = connection.server_commands.encode_frames().unwrap();
        assert!(
            frames
                .windows(b"< 10001 xp 590".len())
                .any(|w| w == b"< 10001 xp 590")
        );
        assert!(frames.windows(b"$ 0 0 1".len()).any(|w| w == b"$ 0 0 1"));
        let sequence = connection.server_commands.sequence();
        connection.server_commands.mark_sent(sequence).unwrap();
        connection.server_commands.acknowledge(sequence).unwrap();
        let (id, revision) = connection.progression_delivery_ack().unwrap();
        services
            .acknowledge_progression(account, &id, revision)
            .await
            .unwrap();
        let reopened = LocalServices::open(temp.path().into()).await.unwrap();
        let retry = reopened
            .commit_match_progression(account, award)
            .await
            .unwrap();
        assert!(retry.duplicate);
        assert_eq!(retry.receipt, committed.receipt);
        let profile = reopened
            .game_profile(account.title_id, account.user_id)
            .await
            .unwrap();
        assert_eq!(profile.rank, (1, 0));
        assert!(
            !reopened
                .pending_progression(account)
                .await
                .unwrap()
                .iter()
                .any(|r| r.match_id == id)
        );
        std::fs::write(root.join("captured-authority-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"native_client_used":false,
            "captured_map":"mp_afghan","real_weapon_shots":fired,"victim":target.client_index,"receipt":committed.receipt,
            "duplicate_retry":true,"reloaded_rank":profile.rank,"notification_acknowledged":true})).unwrap()).unwrap();
    }
    fn connection() -> Connection {
        let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        let template = EnemySnapshot {
            entity_number: 0,
            client_index: 0,
            team: 2,
            model: 4,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: [0.; 3],
            angles: [0; 3],
            legs_animation: 395,
            torso_animation: 395,
            weapon: 352,
            events: Default::default(),
            alive: true,
        };
        let config = MatchConfig {
            mode: crate::content::maps::team_modes::TeamMode::Tactical,
            preparation_msec: 0,
            preparation_text: None,
            grace_period_msec: 0,
            base_respawns: None,
            respawn_points: Vec::new(),
            player_count: 12,
            spawns: [
                (0..6).map(|i| [300., i as f32 * 100., 0.]).collect(),
                (0..6).map(|i| [-300., i as f32 * 100., 0.]).collect(),
            ],
            spawn_angles: None,
            score_limit: 100,
            time_limit_msec: 600000,
            teams: [2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1],
            roster: vec![],
            models: vec![],
        };
        let mut connection = Connection {
            owner_id: 42,
            ..Default::default()
        };
        connection.world.tdm = Some(TdmMatch::new(config, template, 190., &collision).unwrap());
        connection
            .with_progression(
                Policy {
                    kill_player_xp: 100,
                    kill_weapon_xp: 100,
                    headshot_player_bonus: 0,
                },
                &[10001],
            )
            .unwrap()
    }
    fn finish_match(c: &mut Connection) {
        let victim = c
            .world
            .tdm
            .as_ref()
            .unwrap()
            .snapshots()
            .into_iter()
            .find(|s| s.client_index == 6)
            .unwrap();
        c.world.tdm.as_mut().unwrap().player_killed(
            1000,
            victim,
            PlayerHit {
                damage: 100,
                attacker: 0,
                weapon: 352,
                headshot: false,
            },
        );
        let kill = c.world.tdm.as_mut().unwrap().take_kills()[0];
        c.match_progress.as_mut().unwrap().credit(
            kill,
            c.world.tdm.as_ref().unwrap().teams(),
            Some(10001),
        );
    }
    fn presentation_receipt(c: &Connection) -> (crate::backend::progression::Receipt, Vec<u8>) {
        let mut blank = vec![0; 32768];
        let crc = crc32fast::hash(&blank[4..]);
        blank[..4].copy_from_slice(&crc.to_le_bytes());
        let seed = codol_protocol::lobby::profile::prepare_private(&blank, 42, true).unwrap();
        let private =
            codol_protocol::lobby::profile::with_player_progress(&seed, 42, 2, 1).unwrap();
        let tables = crate::backend::progression::Tables::native().unwrap();
        (
            crate::backend::progression::Receipt {
                match_id: c
                    .match_progress
                    .as_ref()
                    .unwrap()
                    .final_award()
                    .unwrap()
                    .match_id,
                revision: 1,
                player: tables.player.award(1901, 100).unwrap(),
                weapons: vec![crate::backend::progression::WeaponResult {
                    item_id: 10001,
                    progress: tables.primary.award(490, 100).unwrap(),
                    eligible_attachments: vec![],
                }],
            },
            private,
        )
    }
    #[test]
    fn full_reliable_window_retries_the_complete_upgrade_batch_and_requires_its_final_ack() {
        let mut c = connection();
        finish_match(&mut c);
        let award = c.pending_match_award().unwrap();
        let (receipt, private) = presentation_receipt(&c);
        for _ in 0..253 {
            c.server_commands.push(b"pending").unwrap();
        }
        let before = c.server_commands.encode_frames().unwrap();
        assert!(c.progression_committed(&receipt, &private).is_err());
        assert_eq!(c.server_commands.encode_frames().unwrap(), before);
        assert_eq!(c.pending_match_award(), Some(award));
        assert!(!c.progression_resolved());
        assert!(c.progression_ack.is_none());
        c.server_commands.mark_sent(253).unwrap();
        c.server_commands.acknowledge(253).unwrap();
        c.progression_committed(&receipt, &private).unwrap();
        let final_sequence = c.server_commands.sequence();
        assert_eq!(final_sequence, 257);
        c.server_commands.mark_sent(final_sequence).unwrap();
        c.server_commands.acknowledge(final_sequence - 1).unwrap();
        assert!(c.progression_delivery_ack().is_none());
        c.server_commands.acknowledge(final_sequence).unwrap();
        assert_eq!(c.progression_delivery_ack(), Some((receipt.match_id, 1)));
    }
    #[test]
    fn full_reliable_window_defers_the_entire_final_scoreboard_until_ack_frees_space() {
        let mut c = connection();
        finish_match(&mut c);
        // This case has no pending account award, so summary publication may
        // proceed independently of progression persistence.
        c.match_progress = None;
        c.world.game_state = Some(Arc::new(
            crate::game::session::prepared::synthetic()
                .with_snapshot_frames(60)
                .unwrap(),
        ));
        let scoreboard = c.world.tdm.as_ref().unwrap().scoreboard(c.owner_id);
        let needed = codol_protocol::game::snapshot::scoreboard::binary_commands(&scoreboard)
            .unwrap()
            .len()
            + codol_protocol::game::snapshot::scoreboard::summary_identity_commands(
                "Player",
                c.world.tdm.as_ref().unwrap().teams(),
            )
            .unwrap()
            .len()
            + 1;
        let backlog = 256 - needed as u32;
        for _ in 0..backlog {
            c.server_commands.push(b"pending").unwrap();
        }
        let before = c.server_commands.encode_frames().unwrap();
        c.publish_match_scoreboard().unwrap();
        assert_eq!(c.server_commands.encode_frames().unwrap(), before);
        assert!(!c.scoreboard_sent);
        assert!(c.world.tdm.as_ref().unwrap().dirty);
        assert!(!c.world.tdm.as_ref().unwrap().result_sent);
        assert!(c.match_summary_sequence.is_none());
        c.challenge = Some(0x75);
        c.gamestate_update = Some(6);
        c.hud_rules_sent = true;
        c.model_rules_sent = true;
        c.world.motion = Some(crate::game::player::movement::Motion::new([0.; 3]));
        // Actual snapshot polling must still transmit the backlog, otherwise
        // the peer cannot ACK it to make room for the deferred summary.
        assert!(c.poll_snapshot(Instant::now()).unwrap().is_some());
        assert_eq!(c.server_commands.encode_frames().unwrap(), before);
        c.server_commands.mark_sent(backlog).unwrap();
        c.server_commands.acknowledge(backlog).unwrap();
        c.publish_match_scoreboard().unwrap();
        let final_sequence = c.server_commands.sequence();
        assert_eq!(final_sequence, backlog + needed as u32);
        assert!(c.scoreboard_sent);
        assert!(!c.world.tdm.as_ref().unwrap().dirty);
        assert!(c.world.tdm.as_ref().unwrap().result_sent);
        assert_eq!(c.match_summary_sequence, Some(final_sequence));
        c.publish_match_scoreboard().unwrap();
        assert_eq!(c.server_commands.sequence(), final_sequence);
        c.server_commands.mark_sent(final_sequence).unwrap();
        c.server_commands.acknowledge(final_sequence - 1).unwrap();
        c.advance_match_summary(2000).unwrap();
        assert!(c.match_ui_sequence.is_none());
        c.server_commands.acknowledge(final_sequence).unwrap();
        for _ in 0..255 {
            c.server_commands.push(b"pending").unwrap();
        }
        let full = c.server_commands.sequence();
        c.advance_match_summary(2050).unwrap();
        assert!(c.match_ui_sequence.is_none());
        assert_eq!(c.server_commands.sequence(), full);
        c.server_commands.mark_sent(full).unwrap();
        c.server_commands.acknowledge(full).unwrap();
        c.advance_match_summary(2100).unwrap();
        assert_eq!(c.match_ui_sequence, Some(full + 1));
        c.server_commands.mark_sent(full + 1).unwrap();
        c.server_commands.acknowledge(full + 1).unwrap();
        c.advance_match_summary(2150).unwrap();
        for _ in 0..255 {
            c.server_commands.push(b"pending").unwrap();
        }
        let full = c.server_commands.sequence();
        c.advance_match_summary(15000).unwrap();
        assert!(c.match_leave_sequence.is_none());
        c.server_commands.mark_sent(full).unwrap();
        c.server_commands.acknowledge(full).unwrap();
        c.advance_match_summary(15050).unwrap();
        assert_eq!(c.match_leave_sequence, Some(full + 1));
        assert_eq!(
            c.server_commands.command_for(full + 1).unwrap(),
            b"t EXE_MATCHENDED"
        );
    }
    #[test]
    fn finished_award_is_stable_retryable_and_requires_ack_before_delivery_confirmation() {
        let mut c = connection();
        assert!(c.pending_match_award().is_none());
        finish_match(&mut c);
        let award = c.pending_match_award().unwrap();
        assert_eq!(c.pending_match_award(), Some(award.clone()));
        assert!(!c.progression_resolved());
        c.progression_commit_failed();
        assert!(c.pending_match_award().is_none());
        c.progression_retry_at = None;
        assert_eq!(c.pending_match_award(), Some(award));
        let (receipt, private) = presentation_receipt(&c);
        c.progression_committed(&receipt, &private).unwrap();
        assert!(c.pending_match_award().is_none());
        assert!(c.progression_resolved());
        let sequence = c.server_commands.sequence();
        c.progression_committed(&receipt, &private).unwrap();
        assert_eq!(c.server_commands.sequence(), sequence);
        assert!(c.progression_delivery_ack().is_none());
        c.server_commands.mark_sent(sequence).unwrap();
        c.server_commands.acknowledge(sequence).unwrap();
        assert_eq!(c.progression_delivery_ack(), Some((receipt.match_id, 1)));
        c.progression_delivery_confirmed();
        assert!(c.progression_delivery_ack().is_none());
    }
}
