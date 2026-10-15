use super::*;

impl PreparedGameState {
    pub(crate) fn model_configstrings(&self) -> Vec<(u16, String)> {
        let mut values = self.match_config.as_ref().map_or_else(Vec::new, |config| {
            config
                .models
                .iter()
                .map(|(id, name)| (2051 + id, name.clone()))
                .collect()
        });
        if let Some(appearance) = &self.player_appearance {
            values.extend(
                appearance
                    .registrations
                    .iter()
                    .map(|(id, name)| (2051 + id, name.clone())),
            );
        }
        values
    }
    pub(crate) fn weapon_configstrings(&self) -> Vec<(u16, String)> {
        if !self.registry_strings.is_empty() {
            return self.registry_strings.clone();
        }
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .chain(&self.match_weapon)
            .chain(&self.roster_weapons)
            .filter_map(|w| {
                w.composed_name
                    .as_ref()
                    .map(|n| (5443 + w.index, n.clone()))
            })
            .collect()
    }
    pub(crate) fn register_weapons(
        &mut self,
        known: &mut std::collections::BTreeMap<String, u16>,
        next: &mut u16,
    ) -> Result<Vec<(u16, String)>> {
        let mut added = Vec::new();
        let first = known.values().min().copied().unwrap_or(*next);
        for weapon in self
            .bootstrap_weapon
            .iter_mut()
            .chain(&mut self.secondary_weapon)
            .chain(&mut self.match_weapon)
            .chain(&mut self.roster_weapons)
        {
            let name = weapon
                .composed_name
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing composed identity"))?;
            let index = if let Some(&index) = known.get(name) {
                index
            } else {
                ensure!(*next < 900, "weapon registry exhausted");
                let index = *next;
                *next += 1;
                known.insert(name.clone(), index);
                added.push((5443 + index, name.clone()));
                index
            };
            weapon.index = index;
            if let Some(ammo) = &mut weapon.ammo {
                ammo.clip_key = index;
            }
            if let Some(base) = weapon.alternate_base {
                // Def+0xc is empty for the target's shared-model stock alts.
                // 0x42dff0 retains the camo and parent composition ordinal,
                // followed by the same parts (format at0xc5da1c/0xc5da2c).
                let alternate = format!(
                    "** {base} {} {}{}",
                    weapon.camo.unwrap_or(0),
                    index - first,
                    weapon
                        .attachments
                        .iter()
                        .map(|id| format!(" {id}"))
                        .collect::<String>()
                );
                let alternate_index = if let Some(&id) = known.get(&alternate) {
                    id
                } else {
                    ensure!(*next < 900, "alternate weapon registry exhausted");
                    let id = *next;
                    *next += 1;
                    known.insert(alternate.clone(), id);
                    added.push((5443 + id, alternate.clone()));
                    id
                };
                if let Some(other) = &mut weapon.alternate {
                    other.index = alternate_index;
                    other.composed_name = Some(alternate);
                    if let Some(ammo) = &mut other.ammo {
                        ammo.clip_key = index;
                    }
                }
            }
        }
        Ok(added)
    }
    pub(crate) fn selected_weapon_names(&self) -> Vec<&str> {
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .filter_map(|w| w.composed_name.as_deref())
            .collect()
    }
    pub(crate) fn same_spawn_equipment(&self, selected: &Self) -> bool {
        let offhands = |state: &Self| {
            state
                .player_grenades
                .each_ref()
                .map(|g| g.as_ref().map(|g| (g.definition.index, g.count)))
        };
        self.selected_weapon_names() == selected.selected_weapon_names()
            && self
                .bootstrap_weapon
                .as_ref()
                .and_then(|w| w.marketplace_item_id)
                == selected
                    .bootstrap_weapon
                    .as_ref()
                    .and_then(|w| w.marketplace_item_id)
            && self
                .secondary_weapon
                .as_ref()
                .and_then(|w| w.marketplace_item_id)
                == selected
                    .secondary_weapon
                    .as_ref()
                    .and_then(|w| w.marketplace_item_id)
            && self.player_perks.slots == selected.player_perks.slots
            && self.player_streak_previews == selected.player_streak_previews
            && offhands(self) == offhands(selected)
    }
    pub(crate) fn register_history(
        &mut self,
        registry: &mut crate::game::session::weapon_registry::WeaponRegistry,
    ) -> Result<()> {
        self.register_weapons(&mut registry.names, &mut registry.next)?;
        self.registry_strings = registry.configstrings();
        self.validate()?;
        Ok(())
    }
    pub(crate) fn body(&self, command_sequence: u32) -> Result<Vec<u8>> {
        self.body_named(command_sequence, None)
    }
    pub(crate) fn body_named(&self, command_sequence: u32, name: Option<&str>) -> Result<Vec<u8>> {
        // CS0 keys mapname/g_gametype confirmed at 0x465aa2/0x465abf.
        // 0x46d830 parses CS3 into C2S serverId. CS16 is map-center text.
        let mut values = vec![
            (
                0,
                format!(
                    "\\mapname\\{}\\g_gametype\\{}\\scr_gameended\\0",
                    self.map, self.mode
                ),
            ),
            (1, String::new()),
            // 0x445b74 compares CS2 with literal "cod" before weapon init.
            (2, "cod".to_owned()),
            (3, "1".to_owned()),
            (16, "0 0 0".to_owned()),
            // 0x579f30 sscanf(CS6415, "%i %i %g %g", ...). Empty text leaves
            // uninitialized slow-motion values and nearly stops client time.
            (6415, "0 0 1 1".to_owned()),
        ];
        if self.training_enemy.is_some() {
            // Current build reads these raw nickname strings, not info pairs.
            values.extend([
                (
                    codol_protocol::game::snapshot::held_weapon::HAND_TAG_CONFIGSTRING,
                    "tag_weapon_right".to_owned(),
                ),
                (1975, name.unwrap_or("Player").to_owned()),
                (1976, "Training Bot".to_owned()),
            ]);
            values.sort_by_key(|value| value.0);
        }
        if let Some(minimap) = self
            .minimap
            .as_ref()
            .or(self.resource_compass_probe.as_ref())
        {
            values.extend(minimap.configstrings());
            values.sort_by_key(|value| value.0);
        }
        if let Some(sound) = self
            .feedback_resources
            .as_ref()
            .and_then(|r| r.sound_configstring())
        {
            values.push(sound);
            values.sort_by_key(|v| v.0);
        }
        if self.match_config.is_some() {
            values.retain(|(index, _)| *index != 1976);
            for client in 1..u16::from(self.match_player_count()) {
                values.push((
                    1975 + client,
                    codol_protocol::room::messages::bot_name(client as u8),
                ));
            }
            values.sort_by_key(|v| v.0);
        }
        // Publish names before cgame/model initialization, not after spawning.
        values.extend(self.model_configstrings());
        if let Some(props) = &self.props_resources {
            values.extend(props.configstrings());
        }
        values.sort_by_key(|v| v.0);
        for (index, name) in self.weapon_configstrings() {
            // NCS inherited weapon bank starts at5443; unused indices are
            // untyped and accept a regular configstring. Parser0x42e3b0.
            values.push((index, name));
            values.sort_by_key(|v| v.0);
        }
        if self.bootstrap_weapon.is_some() {
            // 0x4435fa / 0x5597d0 iterate 1..<CS6800 and register each
            // CG weapon presentation. Without this the secondary's HUD name
            // stays null until viewmodel registration, racing HUD drawing.
            values.push((6800, self.next_weapon_index().to_string()));
            values.sort_by_key(|v| v.0);
        }
        Ok(Gamestate {
            command_sequence,
            map: &self.map,
            zone: &self.zone,
            ncs_crc: self.ncs_crc,
            configstrings: &values,
            client_number: 0,
            checksum_feed: 0,
        }
        .encode()?)
    }
    pub(crate) fn snapshot(
        &self,
        server_time: u32,
        command_time: u32,
        establish_baseline: bool,
        angles: [u16; 3],
        equipment: Option<codol_protocol::game::snapshot::Equipment>,
        world: SnapshotWorld<'_>,
    ) -> Result<Vec<u8>> {
        let motion = world.motion;
        let destroyed_glass = self
            .collision
            .as_ref()
            .map_or_else(Vec::new, |c| c.destroyed_glass());
        // Both roster and local appearance are frozen for this match, so
        // archived views use the same identity even after a backpack refresh.
        let mut characters = [None; 12];
        characters[0] = self.player_appearance.as_ref().and_then(|a| a.character);
        if let Some(config) = &self.match_config {
            for (i, bot) in config.roster.iter().enumerate() {
                characters[i + 1] = bot
                    .outfits
                    .as_ref()
                    .and_then(|outfits| outfits[usize::from(config.teams[i + 1] - 1)].character);
            }
        }
        Ok(codol_protocol::game::snapshot::player_frame_with_world(
            server_time,
            command_time,
            motion.map(|m| m.origin).or(self.spawn_origin),
            establish_baseline,
            equipment,
            angles,
            codol_protocol::game::snapshot::WorldSnapshot {
                weapon_drops: world.weapon_drops,
                weapon_drop_baselines: world.weapon_drop_baselines,
                weapon_hint: world.weapon_hint,
                props: world.props,
                prop_baselines: world.prop_baselines,
                view_offset: world.view_offset,
                streaks: world.streaks,
                radar_flags: world.radar_flags,
                reward_inventory: world.reward_inventory,
                nuke_hud: world.nuke_hud,
                preparation_hud: world.preparation_hud,
                destroyed_glass: &destroyed_glass,
                perk_display: world.perk_display,
                bags: world.bags,
                bag_baselines: world.bag_baselines,
                shellshock: world.shellshock,
                smokes: world.smokes,
                smoke_baselines: world.smoke_baselines,
                missiles: world.missiles,
                missile_baselines: world.missile_baselines,
                local_abilities: world.local_abilities,
                local_abilities_baseline: world.local_abilities_baseline,
                delta_distance: world.delta_distance,
                corpse_baselines: world.corpse_baselines,
                impact_baselines: world.impact_baselines,
                obituary_baselines: world.obituary_baselines,
                target: world.target,
                impacts: world.impacts,
                sounds: world.sounds,
                sound_baselines: world.sound_baselines,
                enemy: world.enemy,
                enemy_baseline: world.enemy_baseline,
                players: world.players,
                player_baselines: world.player_baselines,
                hit_feedback: world.hit_feedback,
                death_waypoints: world.death_waypoints,
                replay: world.replay,
                local_card: self.player_card,
                characters: &characters,
                held_weapon_tag: self
                    .training_enemy
                    .as_ref()
                    .map(|_| codol_protocol::game::snapshot::held_weapon::HAND_TAG_SLOT),
                held_weapons_in_baseline: world.held_weapons_in_baseline,
                vitals: world.vitals,
                local_player: world.local_player,
                local_player_baseline: world.local_player_baseline,
                corpses: world.corpses,
                obituaries: world.obituaries,
                motion: motion.map(|m| codol_protocol::game::snapshot::MotionSnapshot {
                    pm_time: m.pm_time,
                    sprint: m.sprint,
                    velocity: m.velocity,
                    flags: m.flags,
                    ground_entity: m.ground_entity,
                    view_height: m.view_height,
                    view_height_target: m.view_height_target,
                    speed: m.speed,
                    gravity: m.gravity,
                    bob_cycle: m.bob_cycle,
                    view_height_lerp_time: m.view_height_lerp_time,
                    view_height_lerp_target: m.view_height_lerp_target,
                    view_height_lerp_down: m.view_height_lerp_down,
                    jump_time: m.jump_time,
                    jump_origin_z: m.jump_origin_z,
                    melee_charge: m.melee_charge(),
                }),
            },
        )?)
    }
}
