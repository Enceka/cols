//! Resource-bound announcers and authoritative character speech.
//! Nearby team chatter uses the reference GSC's 2s/1000-unit suppression;
//! official 3.3.3.3 timing and package-to-variant assignments remain separate.
use anyhow::{Result, ensure};
use codol_protocol::{
    game_character::Character,
    game_events,
    game_sound::{self, SoundSelector, SoundSnapshot, VoiceFaction, VoiceGender, VoicePrefix},
    game_wire::ReliableCommands,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
};

#[derive(Debug)]
pub(crate) struct Resources {
    sounds: BTreeMap<String, SoundSelector>,
    variant: u8,
    hear_self_chatter: bool,
    mode_intro_gap_msec: u16,
}
#[derive(Deserialize)]
struct Catalog {
    schema_version: u8,
    client_sha256: String,
    map: String,
    mode: String,
    ncs_crc: u32,
    sources_sha256: Vec<String>,
    constant_sound_count: usize,
    #[serde(default = "default_variant")]
    voice_variant: u8,
    #[serde(default)]
    hear_self_chatter: bool,
    #[serde(default = "default_intro_gap")]
    mode_intro_gap_msec: u16,
    all_constant_sounds: Vec<Alias>,
}
fn default_variant() -> u8 {
    1
}
fn default_intro_gap() -> u16 {
    3000
}
#[derive(Deserialize)]
struct Alias {
    alias: String,
    selector: u16,
}
impl Resources {
    pub fn load(path: &Path, map: &str, mode: &str, crc: u32) -> Result<Self> {
        ensure!(
            path.metadata()?.len() <= 2 * 1024 * 1024,
            "voice catalog exceeds budget"
        );
        let raw: Catalog = serde_json::from_slice(&std::fs::read(path)?)?;
        Self::from_catalog(raw, map, mode, crc)
    }
    fn from_catalog(raw: Catalog, map: &str, mode: &str, crc: u32) -> Result<Self> {
        ensure!(
            raw.schema_version == 1
                && raw.client_sha256 == codol_protocol::CLIENT_SHA256
                && raw.map == map
                && raw.mode == mode
                && raw.ncs_crc == crc,
            "voice map/build/CRC mismatch"
        );
        ensure!(
            (2..=3).contains(&raw.sources_sha256.len())
                && raw
                    .sources_sha256
                    .iter()
                    .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "voice sources missing"
        );
        ensure!(
            matches!(raw.voice_variant, 1 | 2),
            "unsupported configured voice variant"
        );
        ensure!(
            (1000..=10000).contains(&raw.mode_intro_gap_msec),
            "invalid mode introduction interval"
        );
        ensure!(
            raw.constant_sound_count == raw.all_constant_sounds.len()
                && (1..=15489).contains(&raw.constant_sound_count),
            "voice constant count mismatch"
        );
        let mut sounds = BTreeMap::new();
        for (index, alias) in raw.all_constant_sounds.into_iter().enumerate() {
            let selector = SoundSelector::constant(index as u16)?;
            ensure!(
                alias.selector == selector.value()
                    && !alias.alias.is_empty()
                    && alias.alias.len() <= 255
                    && alias
                        .alias
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.,/\\-".contains(&b)),
                "invalid voice constant identity"
            );
            ensure!(
                sounds
                    .insert(alias.alias.to_ascii_lowercase(), selector)
                    .is_none(),
                "ambiguous voice constant"
            );
        }
        Ok(Self {
            sounds,
            variant: raw.voice_variant,
            hear_self_chatter: raw.hear_self_chatter,
            mode_intro_gap_msec: raw.mode_intro_gap_msec,
        })
    }
    fn sound(&self, alias: &str) -> Option<SoundSelector> {
        self.sounds.get(&alias.to_ascii_lowercase()).copied()
    }
    pub fn nuke_announcement(&self, team: u8, enemy: bool, earned: bool) -> Option<SoundSelector> {
        let prefix = match team {
            1 => "ab",
            2 => "uk",
            _ => return None,
        };
        let suffix = if earned {
            "achieve_moab"
        } else if enemy {
            "enemy_moab"
        } else {
            "use_moab"
        };
        self.sound(&format!("{prefix}_{}mc_{suffix}", self.variant))
    }
    pub fn prefix(&self, character: Character) -> Option<VoicePrefix> {
        let faction = match character.faction {
            0 => VoiceFaction::TaskForce,
            1 => VoiceFaction::ShadowCompany,
            _ => return None,
        };
        let gender = match character.gender {
            0 => VoiceGender::Male,
            1 => VoiceGender::Female,
            _ => return None,
        };
        let prefix = VoicePrefix::new(faction, gender, self.variant).ok()?;
        self.sound(&prefix.alias("mp_sprint").ok()?).map(|_| prefix)
    }
    fn announcer(&self, team: u8, suffix: &str) -> Option<SoundSelector> {
        let faction = match team {
            1 => "ab",
            2 => "uk",
            _ => return None,
        };
        self.sound(&format!("{faction}_1mc_{suffix}"))
    }
    fn music(&self, team: u8, suffix: &str) -> Option<SoundSelector> {
        let faction = match team {
            1 => "ab",
            2 => "uk",
            _ => return None,
        };
        self.sound(&format!("{faction}_{suffix}_music"))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Actor {
    /// Authoritative birth/respawn identity; a missed death frame must not
    /// let delayed speech from the old life reach the replacement body.
    pub life: u32,
    pub client: u8,
    pub team: u8,
    pub origin: [f32; 3],
    pub alive: bool,
    pub prefix: Option<VoicePrefix>,
    pub events: game_events::EventQueue,
}
#[derive(Clone, Copy)]
struct ActorClock {
    sequence: u8,
    alive: bool,
    generation: u32,
}
struct Delayed {
    due: u32,
    client: u8,
    generation: u32,
}
struct OpeningDialogue {
    boost: SoundSelector,
    mode_sequence: u32,
    acknowledged_at: Option<u32>,
}
#[derive(Default)]
pub(crate) struct Speech {
    prefix_sent: Option<VoicePrefix>,
    started: bool,
    ended: bool,
    opening: Option<OpeningDialogue>,
    clocks: BTreeMap<u8, ActorClock>,
    speakers: BTreeMap<u8, (u32, u8, [f32; 3])>,
    pain_last: BTreeMap<u8, (u32, u32)>,
    pending: Vec<Delayed>,
    active: VecDeque<(u32, SoundSnapshot)>,
    next_entity: u16,
}
impl Speech {
    pub fn prop_sound(&mut self, now: u32, source: u8, origin: [f32; 3], slot: u16) -> Result<()> {
        self.active
            .retain(|(born, _)| now.wrapping_sub(*born) < 200);
        if self.active.len() >= 32 {
            return Ok(());
        }
        let number = if (game_sound::FIRST_SOUND_ENTITY..game_sound::END_SOUND_ENTITY)
            .contains(&self.next_entity)
        {
            self.next_entity
        } else {
            game_sound::FIRST_SOUND_ENTITY
        };
        self.next_entity = if number + 1 == game_sound::END_SOUND_ENTITY {
            game_sound::FIRST_SOUND_ENTITY
        } else {
            number + 1
        };
        self.active.push_back((
            now,
            SoundSnapshot {
                entity_number: number,
                source,
                origin,
                selector: SoundSelector::dynamic(slot)?,
            },
        ));
        Ok(())
    }
    pub fn advance(
        &mut self,
        now: u32,
        resources: &Resources,
        actors: &[Actor],
        ready: bool,
        gameplay: bool,
        commands: &mut ReliableCommands,
    ) -> Result<()> {
        self.active
            .retain(|(born, _)| now.wrapping_sub(*born) < 200);
        self.speakers
            .retain(|_, (born, _, _)| now.wrapping_sub(*born) < 2000);
        self.pain_last
            .retain(|_, (born, _)| now.wrapping_sub(*born) < 2000);
        let local = actors.iter().find(|a| a.client == 0);
        if ready && let Some(local) = local {
            if local.prefix != self.prefix_sent
                && let Some(prefix) = local.prefix
            {
                commands.push(prefix.command().as_bytes())?;
                self.prefix_sent = Some(prefix);
            }
            if gameplay && local.alive && !self.started && !self.ended {
                if let Some(sound) = resources.music(local.team, "spawn") {
                    commands.push(sound.play_command().as_bytes())?;
                }
                if let Some(mode) = resources.announcer(local.team, "tm_deathmatch") {
                    let mode_sequence = commands.push(mode.play_command().as_bytes())?;
                    self.opening =
                        resources
                            .announcer(local.team, "boost")
                            .map(|boost| OpeningDialogue {
                                boost,
                                mode_sequence,
                                acknowledged_at: None,
                            });
                } else if let Some(sound) = resources.announcer(local.team, "boost") {
                    commands.push(sound.play_command().as_bytes())?;
                }
                self.started = true;
            }
        }
        if ready
            && gameplay
            && !self.ended
            && local.is_some_and(|a| a.alive)
            && let Some(opening) = &mut self.opening
            && commands.acknowledged() >= opening.mode_sequence
        {
            let begin = *opening.acknowledged_at.get_or_insert(now);
            if now.wrapping_sub(begin) >= u32::from(resources.mode_intro_gap_msec) {
                commands.push(opening.boost.play_command().as_bytes())?;
                self.opening = None;
            }
        }
        for actor in actors {
            let previous = self.clocks.insert(
                actor.client,
                ActorClock {
                    sequence: actor.events.sequence,
                    alive: actor.alive,
                    generation: actor.life,
                },
            );
            let Some(previous) = previous else {
                continue;
            };
            if !actor.alive || previous.generation != actor.life {
                self.speakers.remove(&actor.client);
                self.pain_last.remove(&actor.client);
            }
            if !ready
                || !gameplay
                || !actor.alive
                || !previous.alive
                || previous.generation != actor.life
            {
                continue;
            }
            let count = actor.events.sequence.wrapping_sub(previous.sequence).min(4);
            for offset in (0..count).rev() {
                let index = actor.events.sequence.wrapping_sub(offset + 1) & 3;
                if actor.events.slots[usize::from(index)].id == game_events::RELOAD_START_NOTIFY {
                    self.say(
                        now,
                        resources,
                        *actor,
                        "inform_reloading_generic",
                        local,
                        true,
                    );
                }
            }
        }
        let pending = std::mem::take(&mut self.pending);
        for delayed in pending {
            if let Some(actor) = actors.iter().find(|a| a.client == delayed.client)
                && actor.alive
                && self
                    .clocks
                    .get(&actor.client)
                    .is_some_and(|c| c.generation == delayed.generation)
            {
                if now.wrapping_sub(delayed.due) > i32::MAX as u32 {
                    if gameplay {
                        self.pending.push(delayed);
                    }
                } else if ready && gameplay {
                    self.say(
                        now,
                        resources,
                        *actor,
                        "inform_killfirm_infantry",
                        local,
                        true,
                    );
                }
            }
        }
        if !gameplay {
            self.pending.clear();
        }
        Ok(())
    }
    pub fn killed(&mut self, now: u32, attacker: u8) {
        if let Some(clock) = self.clocks.get(&attacker).filter(|c| c.alive) {
            self.pending.push(Delayed {
                due: now.wrapping_add(750),
                client: attacker,
                generation: clock.generation,
            });
        }
    }
    pub fn role(
        &mut self,
        now: u32,
        resources: &Resources,
        actor: Actor,
        suffix: &str,
        listener: Option<&Actor>,
        team_others: bool,
    ) {
        self.say(now, resources, actor, suffix, listener, team_others);
    }
    fn say(
        &mut self,
        now: u32,
        resources: &Resources,
        actor: Actor,
        suffix: &str,
        listener: Option<&Actor>,
        team_others: bool,
    ) {
        let Some(listener) = listener else {
            return;
        };
        if self.ended
            || (team_others
                && ((actor.client == listener.client && !resources.hear_self_chatter)
                    || actor.team != listener.team))
        {
            return;
        }
        let Some(selector) = actor
            .prefix
            .and_then(|p| p.alias(suffix).ok())
            .and_then(|name| resources.sound(&name))
        else {
            return;
        };
        if suffix == "mp_pain"
            && self
                .pain_last
                .get(&actor.client)
                .is_some_and(|(born, life)| *life == actor.life && now.wrapping_sub(*born) < 2000)
        {
            return;
        }
        // Team conversation occupancy belongs to battle chatter; pain and
        // death cues do not reserve the team's conversation for two seconds.
        if team_others
            && self.speakers.values().any(|(born, team, origin)| {
                *team == actor.team
                    && now.wrapping_sub(*born) < 2000
                    && distance_squared(*origin, actor.origin) < 1_000_000.0
            })
        {
            return;
        }
        if self.active.len() >= 32 {
            return;
        }
        let number = if !(game_sound::FIRST_SOUND_ENTITY..game_sound::END_SOUND_ENTITY)
            .contains(&self.next_entity)
        {
            game_sound::FIRST_SOUND_ENTITY
        } else {
            self.next_entity
        };
        self.next_entity = if number + 1 == game_sound::END_SOUND_ENTITY {
            game_sound::FIRST_SOUND_ENTITY
        } else {
            number + 1
        };
        self.active.push_back((
            now,
            SoundSnapshot {
                entity_number: number,
                source: actor.client,
                origin: actor.origin,
                selector,
            },
        ));
        if team_others {
            self.speakers
                .insert(actor.client, (now, actor.team, actor.origin));
        } else if suffix == "mp_pain" {
            self.pain_last.insert(actor.client, (now, actor.life));
        }
    }
    pub fn finish(
        &mut self,
        resources: &Resources,
        team: u8,
        winner: u8,
        commands: &mut ReliableCommands,
    ) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        self.pending.clear();
        self.opening = None;
        self.active.clear();
        let (music, dialog) = if winner == 0 {
            (None, "draw")
        } else if winner == team {
            (Some("victory"), "mission_success")
        } else {
            (Some("defeat"), "mission_fail")
        };
        if let Some(sound) = music.and_then(|cue| resources.music(team, cue)) {
            commands.push(sound.play_command().as_bytes())?;
        }
        if let Some(sound) = resources.announcer(team, dialog) {
            commands.push(sound.play_command().as_bytes())?;
        }
        self.ended = true;
        Ok(())
    }
    pub fn snapshots(&self) -> Vec<SoundSnapshot> {
        let mut sounds: Vec<_> = self.active.iter().map(|(_, s)| *s).collect();
        sounds.sort_by_key(|s| s.entity_number);
        sounds
    }
}
fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| (a - b) * (a - b)).sum()
}

pub(crate) fn throw_suffix(name: &str) -> Option<&'static str> {
    match name {
        "frag_grenade_mp"
        | "fragholiday_grenade_mp"
        | "fragshort_grenade_mp"
        | "fragtech_grenade_mp" => Some("inform_attack_grenade"),
        "flash_grenade_mp" => Some("inform_attack_flashbang"),
        "concussion_grenade_mp" => Some("inform_attack_stun"),
        "smoke_grenade_mp" => Some("inform_attack_smoke"),
        "semtex_mp" => Some("mp_semtex_use"),
        "stielhandgranate_mp" => Some("inform_attack_stiel"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resources() -> Resources {
        let names = [
            "uk_spawn_music",
            "uk_victory_music",
            "uk_defeat_music",
            "uk_1mc_boost",
            "uk_1mc_tm_deathmatch",
            "uk_1mc_mission_success",
            "uk_1mc_mission_fail",
            "uk_1mc_draw",
            "uk_m1_mp_sprint",
            "uk_f2_mp_sprint",
            "uk_m1_inform_reloading_generic",
            "uk_m1_inform_killfirm_infantry",
            "uk_m1_mp_death",
            "uk_m1_mp_pain",
            "uk_m1_inform_attack_grenade",
        ];
        Resources {
            variant: 1,
            hear_self_chatter: false,
            mode_intro_gap_msec: 3000,
            sounds: names
                .into_iter()
                .enumerate()
                .map(|(i, n)| (n.to_owned(), SoundSelector::constant(i as u16).unwrap()))
                .collect(),
        }
    }
    fn actor(client: u8, team: u8) -> Actor {
        Actor {
            life: 1,
            client,
            team,
            origin: [f32::from(client) * 10.0, 0.0, 0.0],
            alive: true,
            prefix: Some(
                VoicePrefix::new(
                    if team == 2 {
                        VoiceFaction::TaskForce
                    } else {
                        VoiceFaction::ShadowCompany
                    },
                    VoiceGender::Male,
                    1,
                )
                .unwrap(),
            ),
            events: Default::default(),
        }
    }
    #[test]
    fn announcers_wait_for_readiness_and_start_and_result_are_one_shot() {
        let resources = resources();
        let mut state = Speech::default();
        let mut commands = ReliableCommands::default();
        let mut local = actor(0, 2);
        local.prefix =
            Some(VoicePrefix::new(VoiceFaction::TaskForce, VoiceGender::Female, 2).unwrap());
        state
            .advance(1000, &resources, &[local], false, true, &mut commands)
            .unwrap();
        assert_eq!(commands.sequence(), 0);
        state
            .advance(1050, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(
            commands.command_for(1).unwrap(),
            b"s cg_vo_prefix \"UK_F2_\""
        );
        assert_eq!(
            commands.command_for(2).unwrap(),
            resources
                .music(2, "spawn")
                .unwrap()
                .play_command()
                .as_bytes()
        );
        assert_eq!(commands.sequence(), 3);
        assert_eq!(
            commands.command_for(3).unwrap(),
            resources
                .announcer(2, "tm_deathmatch")
                .unwrap()
                .play_command()
                .as_bytes()
        );
        state
            .advance(1100, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(commands.sequence(), 3);
        commands.mark_sent(3).unwrap();
        commands.acknowledge(3).unwrap();
        state
            .advance(1150, &resources, &[local], true, true, &mut commands)
            .unwrap();
        state
            .advance(4149, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(commands.sequence(), 3);
        state
            .advance(4150, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(
            commands.command_for(4).unwrap(),
            resources
                .announcer(2, "boost")
                .unwrap()
                .play_command()
                .as_bytes()
        );
        state
            .advance(4200, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(commands.sequence(), 4);
        state.finish(&resources, 2, 2, &mut commands).unwrap();
        let count = commands.sequence();
        state.finish(&resources, 2, 2, &mut commands).unwrap();
        assert_eq!(commands.sequence(), count);
        assert_eq!(
            commands.command_for(count).unwrap(),
            resources
                .announcer(2, "mission_success")
                .unwrap()
                .play_command()
                .as_bytes()
        );
        state
            .advance(8000, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(commands.sequence(), count);
    }
    #[test]
    fn reload_wrap_and_retransmission_keep_one_spatial_event_and_team_filter() {
        let resources = resources();
        let mut state = Speech::default();
        let mut commands = ReliableCommands::default();
        let mut actors = [actor(0, 2), actor(1, 2), actor(2, 2), actor(3, 1)];
        actors[1].events.sequence = 254;
        state
            .advance(1000, &resources, &actors, true, true, &mut commands)
            .unwrap();
        for id in [
            game_events::RESET_ADS,
            game_events::RELOAD_START_NOTIFY,
            game_events::RELOAD,
        ] {
            actors[1].events.push(id, 0).unwrap();
        }
        state
            .advance(1100, &resources, &actors, true, true, &mut commands)
            .unwrap();
        let first = state.snapshots();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source, 1);
        assert_eq!(
            first[0].selector,
            resources.sound("uk_m1_inform_reloading_generic").unwrap()
        );
        state
            .advance(1150, &resources, &actors, true, true, &mut commands)
            .unwrap();
        assert_eq!(state.snapshots(), first);
        actors[0]
            .events
            .push(game_events::RELOAD_START_NOTIFY, 0)
            .unwrap();
        actors[2]
            .events
            .push(game_events::RELOAD_START_NOTIFY, 0)
            .unwrap();
        actors[3]
            .events
            .push(game_events::RELOAD_START_NOTIFY, 0)
            .unwrap();
        state
            .advance(1200, &resources, &actors, true, true, &mut commands)
            .unwrap();
        assert_eq!(state.snapshots(), first);
        state
            .advance(1400, &resources, &actors, true, true, &mut commands)
            .unwrap();
        assert!(state.snapshots().is_empty());
        actors[1]
            .events
            .push(game_events::RELOAD_START_NOTIFY, 0)
            .unwrap();
        state
            .advance(3200, &resources, &actors, true, true, &mut commands)
            .unwrap();
        assert_eq!(
            state.snapshots()[0].entity_number,
            first[0].entity_number + 1
        );
    }
    #[test]
    fn ending_the_match_cancels_the_opening_instruction_after_mode_ack() {
        let resources = resources();
        let mut state = Speech::default();
        let mut commands = ReliableCommands::default();
        let local = actor(0, 2);
        state
            .advance(1000, &resources, &[local], true, true, &mut commands)
            .unwrap();
        commands.mark_sent(3).unwrap();
        commands.acknowledge(3).unwrap();
        state
            .advance(1050, &resources, &[local], true, true, &mut commands)
            .unwrap();
        state.finish(&resources, 2, 1, &mut commands).unwrap();
        let count = commands.sequence();
        state
            .advance(10000, &resources, &[local], true, true, &mut commands)
            .unwrap();
        assert_eq!(commands.sequence(), count);
        assert!(state.opening.is_none());
        assert_eq!(
            commands.command_for(count).unwrap(),
            resources
                .announcer(2, "mission_fail")
                .unwrap()
                .play_command()
                .as_bytes()
        );
    }
    #[test]
    fn replay_consumes_old_actions_and_death_cancels_delayed_kill_speech() {
        let resources = resources();
        let mut state = Speech::default();
        let mut commands = ReliableCommands::default();
        let mut actors = [actor(0, 2), actor(1, 2)];
        state
            .advance(1000, &resources, &actors, true, true, &mut commands)
            .unwrap();
        state.killed(1050, 1);
        actors[1].alive = false;
        state
            .advance(1100, &resources, &actors, true, true, &mut commands)
            .unwrap();
        actors[1].alive = true;
        actors[1]
            .events
            .push(game_events::RELOAD_START_NOTIFY, 0)
            .unwrap();
        state
            .advance(1700, &resources, &actors, true, false, &mut commands)
            .unwrap();
        state
            .advance(1900, &resources, &actors, true, true, &mut commands)
            .unwrap();
        assert!(state.snapshots().is_empty());
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_missed_death_frame_still_cancels_speech_owned_by_the_previous_life() {
        let resources = resources();
        let mut state = Speech::default();
        let mut commands = ReliableCommands::default();
        let mut actors = [actor(0, 2), actor(1, 2)];
        state
            .advance(1000, &resources, &actors, true, true, &mut commands)
            .unwrap();
        state.killed(1100, 1);
        actors[1].life += 1;
        state
            .advance(2000, &resources, &actors, true, true, &mut commands)
            .unwrap();
        assert!(state.snapshots().is_empty());
        assert!(state.pending.is_empty());
    }

    #[test]
    fn grenade_self_chatter_requires_explicit_configuration_and_known_alias() {
        let mut resources = resources();
        let mut speech = Speech::default();
        let local = actor(0, 2);
        let cue = throw_suffix("frag_grenade_mp").unwrap();
        speech.role(1000, &resources, local, cue, Some(&local), true);
        assert!(speech.snapshots().is_empty());
        resources.hear_self_chatter = true;
        speech.role(1000, &resources, local, cue, Some(&local), true);
        assert_eq!(speech.snapshots()[0].source, 0);
        assert!(throw_suffix("emp_grenade_mp").is_none());
        assert!(throw_suffix("tabun_gas_mp").is_none());
    }
    #[test]
    fn pain_is_throttled_per_life_without_silencing_team_grenade_calls() {
        let mut resources = resources();
        resources.hear_self_chatter = true;
        let mut speech = Speech::default();
        let local = actor(0, 2);
        speech.role(1000, &resources, local, "mp_pain", Some(&local), false);
        speech.role(
            1100,
            &resources,
            local,
            "inform_attack_grenade",
            Some(&local),
            true,
        );
        assert_eq!(speech.snapshots().len(), 2);
        speech.role(1200, &resources, local, "mp_pain", Some(&local), false);
        assert_eq!(speech.snapshots().len(), 2);
        let mut reborn = local;
        reborn.life += 1;
        speech.role(1250, &resources, reborn, "mp_pain", Some(&reborn), false);
        assert_eq!(speech.snapshots().len(), 3);
    }
    #[test]
    fn configured_variant_and_catalog_identity_must_be_valid() {
        let raw = || Catalog {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.into(),
            map: "mp_afghan".into(),
            mode: "war".into(),
            ncs_crc: 123,
            sources_sha256: vec!["0".repeat(64); 2],
            constant_sound_count: 1,
            voice_variant: 2,
            hear_self_chatter: false,
            mode_intro_gap_msec: 3000,
            all_constant_sounds: vec![Alias {
                alias: "uk_f2_mp_sprint".into(),
                selector: 512,
            }],
        };
        let r = Resources::from_catalog(raw(), "mp_afghan", "war", 123).unwrap();
        let c = Character {
            gender: 1,
            faction: 0,
            skin: 0,
            groups: [0; 11],
            bits: [0; 11],
        };
        assert_eq!(r.prefix(c).unwrap().prefix(), "UK_F2_");
        assert!(Resources::from_catalog(raw(), "mp_bog_sh", "war", 123).is_err());
        assert!(Resources::from_catalog(raw(), "mp_afghan", "war", 124).is_err());
        let mut shifted = raw();
        shifted.all_constant_sounds[0].selector += 1;
        assert!(Resources::from_catalog(shifted, "mp_afghan", "war", 123).is_err());
        let mut variant = raw();
        variant.voice_variant = 3;
        assert!(Resources::from_catalog(variant, "mp_afghan", "war", 123).is_err());
    }
}
