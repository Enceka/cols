use super::*;
use crate::game::world::weapon_drops::User;

impl Connection {
    pub(super) fn drop_death_weapon(&mut self, kill: crate::game::modes::tdm::Kill) -> bool {
        let tdm = self.world.tdm.as_mut().unwrap();
        if tdm.in_grace_period(kill.time) {
            return false;
        }
        let weapon = if kill.victim.client_index == 0 {
            self.world
                .player
                .as_mut()
                .and_then(|p| p.take_death_weapon())
        } else {
            tdm.take_death_weapon(kill.victim.client_index)
        };
        let Some(weapon) = weapon else {
            return false;
        };
        let observers: Vec<_> = tdm
            .snapshots()
            .iter()
            .filter(|p| p.alive)
            .map(|p| p.origin)
            .chain(
                self.world
                    .motion
                    .iter()
                    .filter(|_| self.world.vitals.health > 0)
                    .map(|m| m.origin),
            )
            .collect();
        self.world.weapon_drops.spawn(
            kill.time,
            kill.victim.client_index,
            kill.victim.origin,
            kill.victim.angles,
            weapon,
            &observers,
        );
        true
    }
    fn ground_weapon_user(&self, now: u32) -> Option<User> {
        let motion = self.world.motion.as_ref()?;
        let angles = self.view_angles();
        let yaw = f32::from(angles[1]) * std::f32::consts::TAU / 65536.0;
        let pitch = f32::from(angles[0] as i16) * std::f32::consts::TAU / 65536.0;
        Some(User {
            actor: 0,
            feet: motion.origin,
            eye: [
                motion.origin[0],
                motion.origin[1],
                motion.origin[2] + motion.view_height,
            ],
            forward: [
                pitch.cos() * yaw.cos(),
                pitch.cos() * yaw.sin(),
                -pitch.sin(),
            ],
            active: self.world.vitals.health > 0
                && !self.world.armor.active()
                && !self.replay.pending()
                && self.world.tdm.as_ref().is_some_and(|m| m.live_at(now)),
            use_button: self.last_input.as_ref().is_some_and(|cmd| {
                now.saturating_sub(cmd.time) < 200
                    && cmd.buttons & codol_protocol::game::channel::usercmd::BUTTON_USE != 0
            }),
        })
    }
    pub(super) fn advance_weapon_drops(&mut self, now: u32) -> Result<()> {
        if self.world.tdm.as_ref().is_none_or(|m| m.finished()) {
            self.world.weapon_drops.clear();
            return Ok(());
        }
        let Some(user) = self.ground_weapon_user(now) else {
            return Ok(());
        };
        let state = self.world.game_state.as_ref().unwrap();
        let collision = state.collision.as_ref().unwrap();
        self.world.weapon_drops.advance(now, collision);
        // Selection and ownership must commit together. Retry the held use
        // after ACKs free a slot rather than changing guns without 'a'.
        if self.server_commands.available() == 0 {
            return Ok(());
        }
        if self.world.weapon_drops.interact(
            now,
            user,
            self.world.player.as_mut().unwrap(),
            collision,
        ) {
            // Native 'a' corrects CG's selected gun as well as the snapshot PS.
            let weapon = self.world.player.as_ref().unwrap().equipment().weapon;
            self.server_commands
                .push(format!("a {weapon}").as_bytes())?;
            if let Some(motion) = &mut self.world.motion {
                motion.cancel_sprint();
            }
        }
        Ok(())
    }
    pub(super) fn ground_weapon_hint(
        &self,
        now: u32,
    ) -> Option<codol_protocol::game::snapshot::weapon_drops::WeaponHint> {
        self.world.weapon_drops.hint(
            now,
            self.ground_weapon_user(now)?,
            self.world.player.as_ref()?,
            self.world.game_state.as_ref()?.collision.as_ref()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codol_protocol::game::channel::usercmd::UserCommand;

    #[test]
    #[ignore = "uses locally captured Afghan collision, arsenal and a read-only local profile"]
    fn captured_shooting_death_drop_and_use_exchange_preserve_real_weapon_identity() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let source = root.join("nuke-acceptance-20261013/map.json");
        let mut map: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
        map.as_object_mut().unwrap().remove("nuke_file");
        map["match_config"]["preparation_msec"] = 0.into();
        map["match_config"]["grace_period_msec"] = 0.into();
        map["spawn_origin"] = map["match_config"]["spawns"][1][0].clone();
        for (key, value) in map.as_object_mut().unwrap().iter_mut() {
            if key.ends_with("_file")
                && let Some(name) = value.as_str()
            {
                *value = source
                    .parent()
                    .unwrap()
                    .join(name)
                    .to_string_lossy()
                    .into_owned()
                    .into();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("map.json");
        std::fs::write(&path, serde_json::to_vec(&map).unwrap()).unwrap();
        let template = crate::game::session::prepared::PreparedGameState::load(&path).unwrap();
        let profiles: Vec<_> =
            std::fs::read_dir(root.join("../client-work/server-data/profiles/1002"))
                .unwrap()
                .map(|p| p.unwrap().path())
                .filter(|p| p.join("public.bin").is_file() && p.join("inventory.json").is_file())
                .collect();
        assert_eq!(profiles.len(), 1, "capture requires one local profile");
        let public = std::fs::read(profiles[0].join("public.bin")).unwrap();
        let inventory: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profiles[0].join("inventory.json")).unwrap())
                .unwrap();
        let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
        let state = Arc::new(
            template
                .for_initial_backpack(&public)
                .unwrap()
                .for_profile(&public, &owned)
                .unwrap(),
        );
        let mut c = Connection::with_game_state(state.clone());
        let local = state.local_player_snapshot(true).unwrap();
        let eye = [local.origin[0], local.origin[1], local.origin[2] + 60.0];
        let targets = c.world.tdm.as_ref().unwrap().snapshots();
        let target = *targets
            .iter()
            .filter(|p| p.team != local.team)
            .min_by(|a, b| {
                let distance = |p: &&codol_protocol::game::snapshot::enemy::EnemySnapshot| {
                    (p.origin[0] - eye[0]).hypot(p.origin[1] - eye[1])
                };
                distance(a).total_cmp(&distance(b))
            })
            .unwrap();
        let delta = [
            target.origin[0] - eye[0],
            target.origin[1] - eye[1],
            target.origin[2] + 35.0 - eye[2],
        ];
        let angle = |r: f32| {
            ((r.rem_euclid(std::f32::consts::TAU) * 65536.0 / std::f32::consts::TAU).round() as u32
                & 65535) as u16
        };
        let angles = [
            angle(-delta[2].atan2(delta[0].hypot(delta[1]))),
            angle(delta[1].atan2(delta[0])),
            0,
        ];
        let old_index = c.world.player.as_ref().unwrap().equipment().weapon;
        let mut death_time = 0;
        for time in (1100..=6100).step_by(100) {
            c.world.player.as_mut().unwrap().apply(&UserCommand {
                time,
                buttons: 0x801,
                angles,
                weapons: [old_index, 0, 0],
                ..Default::default()
            });
            for shot in c.world.player.as_mut().unwrap().take_shots() {
                c.world.tdm.as_mut().unwrap().player_fire(
                    shot,
                    eye,
                    local,
                    state.collision.as_ref().unwrap(),
                    None,
                );
            }
            c.drain_match_kills(time);
            if !c.world.weapon_drops.snapshots().is_empty() {
                death_time = time;
                break;
            }
        }
        assert!(
            death_time > 0,
            "real bullets must kill an enemy and produce its gun"
        );
        let dropped = c.world.weapon_drops.snapshots()[0];
        assert_eq!(dropped.weapon, target.weapon);
        assert!(
            c.corpses
                .values()
                .any(|(_, corpse)| corpse.donor_client == target.client_index
                    && corpse.weapon_dropped)
        );
        assert_ne!(
            dropped.weapon, old_index,
            "killer's gun is not the death gun"
        );
        c.drain_match_kills(death_time);
        assert_eq!(c.world.weapon_drops.snapshots().len(), 1);
        let now = death_time + 2000;
        c.world
            .weapon_drops
            .advance(now, state.collision.as_ref().unwrap());
        let landed = c.world.weapon_drops.snapshots()[0];
        let feet = crate::game::ai::controller::grounded_spawn(
            [
                landed.origin[0],
                landed.origin[1] - 48.0,
                landed.origin[2] + 20.0,
            ],
            state.collision.as_ref().unwrap(),
        )
        .unwrap();
        c.world.motion.as_mut().unwrap().origin = feet;
        c.world.player.as_mut().unwrap().apply(&UserCommand {
            time: now,
            buttons: 0,
            weapons: [old_index, 0, 0],
            ..Default::default()
        });
        let old_ammo = c.world.player.as_ref().unwrap().equipment().ammo.unwrap();
        c.last_input = Some(UserCommand {
            time: now,
            buttons: 0,
            angles: [0, 16384, 0],
            ..Default::default()
        });
        let hint = c
            .ground_weapon_hint(now)
            .expect("real collision must allow nearby use hint");
        assert_eq!(hint.weapon, target.weapon);
        c.last_input.as_mut().unwrap().buttons = codol_protocol::game::channel::usercmd::BUTTON_USE;
        while c.server_commands.available() > 0 {
            c.server_commands.push(b"pending").unwrap();
        }
        c.advance_weapon_drops(now).unwrap();
        assert_eq!(
            c.world.player.as_ref().unwrap().equipment().weapon,
            old_index
        );
        let sent = c.server_commands.sequence();
        c.server_commands.mark_sent(sent).unwrap();
        c.server_commands.acknowledge(sent).unwrap();
        c.advance_weapon_drops(now).unwrap();
        let equipped = c.world.player.as_ref().unwrap().equipment();
        assert_eq!(equipped.weapon, target.weapon);
        assert!(!c.world.player.as_ref().unwrap().owns_weapon(old_index));
        assert_eq!(c.world.weapon_drops.snapshots()[0].weapon, old_index);
        c.advance_weapon_drops(now + 50).unwrap();
        assert_eq!(
            c.world.player.as_ref().unwrap().equipment().weapon,
            target.weapon
        );
        let snapshots = c.world.weapon_drops.snapshots();
        state
            .snapshot(
                now + 50,
                now,
                false,
                [0, 16384, 0],
                Some(equipped),
                crate::game::session::prepared::SnapshotWorld {
                    weapon_drops: &snapshots,
                    motion: c.world.motion.as_ref(),
                    ..Default::default()
                },
            )
            .unwrap();
        let proof = serde_json::json!({"client_sha256":codol_protocol::CLIENT_SHA256,"native_gameplay_acceptance":false,
            "captured_collision_gameplay":true,"death_time":death_time,"victim":target.client_index,"killer_weapon":old_index,
            "death_weapon":target.weapon,"ground_entity":landed.entity_number,"landed_origin":landed.origin,
            "hint_weapon":hint.weapon,"hint_entity":hint.entity,"old_clip_after_fire":old_ammo.clip,
            "picked_clip":equipped.ammo.unwrap().clip,"picked_reserve":equipped.ammo.unwrap().reserve,
            "old_weapon_returned_to_ground":snapshots[0].weapon,"repeat_use_kept_weapon":true});
        let evidence = root.join("weapon-drops-20261015");
        std::fs::create_dir_all(&evidence).unwrap();
        std::fs::write(
            evidence.join("captured-gameplay-proof.json"),
            serde_json::to_vec_pretty(&proof).unwrap(),
        )
        .unwrap();
    }
}
