use super::*;

impl PreparedGameState {
    pub(crate) fn enemy_snapshot(
        &self,
        alive: bool,
    ) -> Option<codol_protocol::game::snapshot::enemy::EnemySnapshot> {
        self.training_enemy.as_ref().map(|enemy| {
            let mut snapshot = enemy.snapshot(alive);
            if let Some(weapon) = &self.bootstrap_weapon {
                snapshot.weapon = weapon.index;
            }
            snapshot
        })
    }
    pub(crate) fn local_player_snapshot(
        &self,
        alive: bool,
    ) -> Option<codol_protocol::game::snapshot::enemy::EnemySnapshot> {
        let mut player = self.enemy_snapshot(alive)?;
        player.entity_number = 0;
        player.client_index = 0;
        player.team = self.match_config.as_ref().map_or(2, |c| c.teams[0]);
        // Captured complete standard male skeleton. This training appearance
        // is independent of the already live first-person backpack equipment.
        player.model = 4;
        player.attachment_models = [3, 2, 5, 6, 0, 0, 0, 0, 0, 0];
        player.attachment_tags = [0; 10];
        if let Some(appearance) = &self.player_appearance {
            player.model = appearance.indices[0];
            player.attachment_models = [0; 10];
            for (slot, &index) in player
                .attachment_models
                .iter_mut()
                .zip(&appearance.indices[1..])
            {
                *slot = index;
            }
        }
        player.origin = self.spawn_origin?;
        player.angles = [0; 3];
        Some(player)
    }
    pub(crate) fn corpse(
        &self,
        player: codol_protocol::game::snapshot::enemy::EnemySnapshot,
        sequence: u32,
    ) -> codol_protocol::game::snapshot::corpse::CorpseSnapshot {
        let animation = self.training_enemy.as_ref().unwrap().death_animation;
        codol_protocol::game::snapshot::corpse::CorpseSnapshot {
            entity_number: if player.client_index == 0 { 20 } else { 19 },
            donor_client: player.client_index,
            origin: player.origin,
            angles: player.angles,
            legs_animation: animation,
            torso_animation: animation,
            weapon: player.weapon,
            present: true,
            phase: codol_protocol::game::snapshot::corpse::CorpsePhase::Playing,
            death_sequence: sequence,
        }
    }
    pub(crate) fn bot(&self) -> Option<crate::game::ai::controller::BotController> {
        if self.is_match() {
            return None;
        }
        self.training_enemy
            .as_ref()
            .filter(|enemy| enemy.bot)
            .map(|enemy| {
                crate::game::ai::controller::BotController::new(
                    crate::game::ai::controller::grounded_spawn(
                        enemy.origin,
                        self.collision.as_ref().unwrap(),
                    )
                    .expect("validated bot spawn"),
                    enemy.angles,
                    enemy.move_speed,
                )
                .expect("validated bot configuration")
            })
    }
    pub(crate) fn bot_weapon(&self) -> Option<crate::game::ai::controller::BotWeapon> {
        let weapon = self.bootstrap_weapon.as_ref()?;
        Some(crate::game::ai::controller::BotWeapon {
            weapon: weapon.index,
            ballistics: weapon.ballistics?,
            fire_interval_msec: u32::from(weapon.ammo.as_ref()?.fire_interval_msec?),
        })
    }
    pub(crate) fn animate_enemy(
        &self,
        enemy: codol_protocol::game::snapshot::enemy::EnemySnapshot,
        frame: &crate::game::ai::controller::BotFrame,
        firing: bool,
        toggle: bool,
    ) -> codol_protocol::game::snapshot::enemy::EnemySnapshot {
        let config = self
            .training_enemy
            .as_ref()
            .expect("enemy frame requires configuration");
        Self::animate_with_config(config, enemy, frame, firing, toggle)
    }
    pub(crate) fn animate_with_config(
        config: &crate::game::ai::enemy::EnemyConfig,
        mut enemy: codol_protocol::game::snapshot::enemy::EnemySnapshot,
        frame: &crate::game::ai::controller::BotFrame,
        firing: bool,
        toggle: bool,
    ) -> codol_protocol::game::snapshot::enemy::EnemySnapshot {
        enemy.origin = frame.motion.origin;
        enemy.angles = frame.angles;
        let moving = frame.moving();
        let direction = if moving {
            let yaw = f32::from(frame.angles[1]) * std::f32::consts::TAU / 65536.0;
            let (sin, cos) = yaw.sin_cos();
            let forward = frame.motion.velocity[0] * cos + frame.motion.velocity[1] * sin;
            let right = -frame.motion.velocity[0] * sin + frame.motion.velocity[1] * cos;
            if right.abs() > forward.abs() {
                if right > 0.0 { 2 } else { 3 }
            } else if forward < 0.0 {
                1
            } else {
                0
            }
        } else {
            0
        };
        enemy.legs_animation = if moving {
            if let Some(a) = &config.locomotion {
                if frame.locomotion == crate::game::ai::controller::Locomotion::Sprint {
                    a.sprint
                } else if frame.locomotion == crate::game::ai::controller::Locomotion::Run {
                    [a.run_forward, a.run_back, a.run_left, a.run_right][direction]
                } else {
                    [
                        Some(config.walk_animation),
                        config.walk_back_animation,
                        config.walk_left_animation,
                        config.walk_right_animation,
                    ][direction]
                        .unwrap_or(config.walk_animation)
                }
            } else {
                [
                    Some(config.walk_animation),
                    config.walk_back_animation,
                    config.walk_left_animation,
                    config.walk_right_animation,
                ][direction]
                    .unwrap_or(config.walk_animation)
            }
        } else {
            config.idle_animation
        };
        // Current playeranim.script's fireweapon/movetype moving branch is
        // empty: the full-body locomotion already holds the gun correctly.
        enemy.torso_animation = if firing && !(moving && config.locomotion.is_some()) {
            config
                .locomotion
                .as_ref()
                .map_or(config.fire_animation, |a| a.fire_standing)
                | if toggle { 0x400 } else { 0 }
        } else {
            0 // The legs node carries the full-body pose; torso0 keeps
            // an additional idle node from overriding its moving legs.
        };
        enemy
    }
    pub(crate) fn player(&self) -> Option<crate::game::weapons::held::HeldWeapons> {
        self.primary_player()
            .map(|p| match self.secondary_player() {
                Some(s) => p.with_secondary(s),
                None => p,
            })
    }
    pub(crate) fn primary_player(&self) -> Option<crate::game::weapons::held::HeldWeapons> {
        self.bootstrap_weapon
            .as_ref()
            .map(|w| self.playable_weapon(w))
    }
    pub(crate) fn secondary_player(&self) -> Option<crate::game::weapons::held::HeldWeapons> {
        self.secondary_weapon
            .as_ref()
            .map(|w| self.playable_weapon(w))
    }
    pub(super) fn playable_weapon(
        &self,
        weapon: &BootstrapWeapon,
    ) -> crate::game::weapons::held::HeldWeapons {
        let mut player = Self::make_player(weapon);
        player.set_perks(self.player_perks.clone());
        player.set_offhands(self.player_grenades.clone());
        // The ordinary melee damage path currently belongs to TDM. Legacy
        // prop/pose diagnostics must not emit an animation with no resolver.
        if !self.is_match() {
            player.disable_melee();
            player.set_offhands([None, None]);
        }
        player
    }
    pub(super) fn make_player(w: &BootstrapWeapon) -> crate::game::weapons::held::HeldWeapons {
        let mut player = crate::game::weapons::held::HeldWeapons::new(
            codol_protocol::game::snapshot::Equipment {
                perks: Default::default(),
                abilities: Default::default(),
                offhands: [None, None],
                offhand_weapon: 0,
                grenade_time: 0,
                move_multiplier: 1.0,
                inventory_weapon: None,
                alternate_weapon: None,
                alternates: [None, None],
                fire_count: 0,
                queued_burst: false,
                barrel_flags: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
                secondary: None,
                events: None,
                weapon: w.index,
                viewmodel: w.viewmodel_index,
                firing: None,
                aim: None,
                ammo: w
                    .ammo
                    .as_ref()
                    .map(|a| codol_protocol::game::snapshot::Ammo {
                        clip_key: a.clip_key,
                        reserve_key: a.reserve_key,
                        clip: a.clip,
                        reserve: a.reserve,
                    }),
            },
            w.ammo.as_ref().and_then(|a| a.fire_interval_msec),
            w.ammo.as_ref().and_then(|a| a.reload),
            w.ammo.as_ref().and_then(|a| a.ads),
        );
        if let Some(timing) = w.switch {
            player.set_switch_timing(timing);
        }
        player.set_fast_reload(w.fast_reload);
        if let Some(limits) = w.ammo.as_ref().and_then(|a| a.limits) {
            player.set_ammo_limits(limits);
        }
        player.set_offhand_switch(w.offhand_switch);
        if let Some(timing) = w.rechamber {
            player.set_rechamber_timing(timing);
        }
        if let Some(timing) = w.spread {
            player.set_spread_timing(timing);
        }
        if let Some(timing) = w.melee {
            player.set_melee_timing(timing);
        }
        if let Some(scale) = w.move_scale {
            player.set_move_scale(scale);
        }
        if let Some(scale) = w.ads_move_scale {
            player.set_ads_move_scale(scale);
        }
        if let Some(scale) = w.sprint_duration_scale {
            player.set_sprint_duration_scale(scale);
        }
        if let Some(timing) = w.sprint_timing {
            player.set_sprint_timing(timing);
        }
        if let Some(kind) = w.fire_type {
            player.set_fire_type(kind).expect("validated fire type");
        }
        if let Some(stats) = w.ballistics {
            player.set_ballistics(stats);
        }
        if let Some(alternate) = &w.alternate {
            player =
                player.with_alternate(Self::make_player(alternate), w.alternate_timing.unwrap());
        }
        player
    }
    pub(crate) fn motion(&self) -> Option<crate::game::player::movement::Motion> {
        self.collision
            .as_ref()
            .and(self.spawn_origin)
            .map(|origin| {
                let mut m = crate::game::player::movement::Motion::new(origin);
                m.command_time = 1000;
                m
            })
    }
    pub(crate) fn combat(&self) -> Option<crate::game::combat::CombatWorld> {
        if self.match_config.is_some() {
            return None;
        }
        self.training_target
            .clone()
            .or_else(|| self.training_enemy.as_ref().map(|enemy| enemy.target()))
            .map(crate::game::combat::CombatWorld::new)
    }
    pub(crate) fn input_base(&self) -> codol_protocol::game::channel::usercmd::UserCommand {
        let mut base = codol_protocol::game::channel::usercmd::UserCommand::default();
        if let Some(weapon) = &self.bootstrap_weapon {
            base.weapons[0] = weapon.index;
            base.melee_entity = 2047; // target 0x5a18e0, POF_PLAYER (0x1000)
        }
        base
    }
    pub(crate) fn tdm_match(&self) -> Result<Option<crate::game::modes::tdm::TdmMatch>> {
        let Some(config) = &self.match_config else {
            return Ok(None);
        };
        let mut template = self
            .local_player_snapshot(true)
            .ok_or_else(|| anyhow::anyhow!("TDM missing player appearance"))?;
        template.weapon = self
            .match_weapon
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TDM bot rifle missing"))?
            .index;
        let mut tdm = crate::game::modes::tdm::TdmMatch::with_navigation(
            config.clone(),
            template,
            self.training_enemy.as_ref().unwrap().move_speed,
            self.collision.as_ref().unwrap(),
            self.navigation
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("TDM navigation not prepared"))?
                .clone(),
        )?;
        tdm.configure_roster(self.roster_weapons.iter().map(Self::make_player).collect())?;
        if let Some(resources) = &self.props_resources {
            tdm.props = crate::game::world::props::Props::new(
                resources.clone(),
                self.collision.as_ref().unwrap(),
            );
        }
        if let Some(resources) = &self.radar_resources {
            tdm.streaks.enable_supported(resources);
            tdm.streaks.configure_radios(0, self.player_streak_previews);
        }
        if let Some(resources) = &self.nuke_resources {
            tdm.streaks.enable();
            tdm.streaks.acceptance_seed(resources.acceptance_seed);
            tdm.acceptance_stationary = resources.acceptance_stationary;
            tdm.acceptance_fire_after_msec = resources.acceptance_fire_after_msec;
        }
        tdm.set_weapon_move_scale(
            self.match_weapon
                .as_ref()
                .and_then(|w| w.move_scale)
                .unwrap_or(1.0),
        );
        Ok(Some(tdm))
    }
}
