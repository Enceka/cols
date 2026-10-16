//! One real PLAYER/client1 assembled from captured human assets and animations.
use anyhow::{Result, ensure};
use codol_protocol::game::snapshot::enemy::EnemySnapshot;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnemyConfig {
    pub model: u16,
    pub origin: [f32; 3],
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub idle_animation: u16,
    pub walk_animation: u16,
    #[serde(default)]
    pub walk_left_animation: Option<u16>,
    #[serde(default)]
    pub walk_right_animation: Option<u16>,
    #[serde(default)]
    pub walk_back_animation: Option<u16>,
    #[serde(default)]
    pub locomotion: Option<NativeLocomotion>,
    pub fire_animation: u16,
    #[serde(default)]
    pub fire_animation_msec: Option<u16>,
    #[serde(default)]
    pub reload_animation: Option<u16>,
    pub death_animation: u16,
    pub move_speed: f32,
    pub angles: [u16; 3],
    pub weapon: u16,
    pub source_sha256: String,
    #[serde(default)]
    pub bot: bool,
}

/// Bound to the captured3.3 script's autorifle rules and human registry.
/// Physical speed still comes from Pmove/WeaponDef rather than root motion.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeLocomotion {
    pub run_forward: u16,
    pub run_back: u16,
    pub run_left: u16,
    pub run_right: u16,
    pub sprint: u16,
    pub fire_standing: u16,
    pub script_sha256: String,
}

impl EnemyConfig {
    pub fn validate(&self) -> Result<()> {
        self.target().validate()?;
        ensure!(
            self.fire_animation_msec
                .is_none_or(|t| (1..=15000).contains(&t)),
            "invalid native firing animation duration"
        );
        if let Some(a) = &self.locomotion {
            ensure!(
                [
                    a.run_forward,
                    a.run_back,
                    a.run_left,
                    a.run_right,
                    a.sprint,
                    a.fire_standing
                ]
                .into_iter()
                .all(|i| (1..1024).contains(&i))
                    && a.script_sha256.len() == 64
                    && a.script_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid native locomotion capture"
            );
        }
        ensure!(
            [
                self.walk_left_animation,
                self.walk_right_animation,
                self.walk_back_animation,
                self.reload_animation,
            ]
            .into_iter()
            .flatten()
            .all(|a| (1..1024).contains(&a)),
            "invalid directional walk animation"
        );
        ensure!(
            (1..900).contains(&self.weapon),
            "enemy weapon outside registry"
        );
        ensure!(
            [
                self.idle_animation,
                self.walk_animation,
                self.fire_animation,
                self.death_animation
            ]
            .into_iter()
            .all(|a| (1..1024).contains(&a)),
            "invalid captured human animation"
        );
        ensure!(
            self.move_speed.is_finite() && (20.0..=300.0).contains(&self.move_speed),
            "invalid captured enemy movement speed"
        );
        Ok(())
    }

    pub fn target(&self) -> crate::game::combat::TargetConfig {
        crate::game::combat::TargetConfig {
            entity_number: 1,
            model: self.model,
            origin: self.origin,
            scale: 1.0,
            bounds_min: self.bounds_min,
            bounds_max: self.bounds_max,
            health: 100,
            respawn_msec: 3000,
            source_sha256: self.source_sha256.clone(),
        }
    }

    pub fn snapshot(&self, alive: bool) -> EnemySnapshot {
        EnemySnapshot {
            entity_number: 1,
            client_index: 1,
            team: 1,
            model: self.model,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: self.origin,
            angles: self.angles,
            legs_animation: self.idle_animation,
            torso_animation: self.idle_animation,
            weapon: self.weapon,
            alive,
            events: Default::default(),
        }
    }
}

/// Overlay an actual weapon reload while retaining the locomotion legs.
/// Weapon animation identities change at shell/start/end transitions. The
/// native body restart bit is independent of the weapon/viewmodel toggle.
#[derive(Default)]
pub(crate) struct ReloadAnimation {
    key: Option<(u16, u16)>,
    toggle: bool,
}
impl ReloadAnimation {
    pub fn apply(
        &mut self,
        actor: &mut EnemySnapshot,
        animation: Option<u16>,
        equipment: Option<codol_protocol::game::snapshot::Equipment>,
    ) -> bool {
        let reload = equipment.and_then(|e| {
            e.firing.filter(|f| (8..=12).contains(&f.state)).map(|f| {
                // Closing a segmented reload is not another insertion.
                // Keep the preceding body action through its final phase.
                if f.state == 12 {
                    self.key
                        .filter(|k| k.0 == e.weapon)
                        .unwrap_or((e.weapon, f.animation))
                } else {
                    (e.weapon, f.animation)
                }
            })
        });
        if let (Some(animation), Some(key)) = (animation, reload) {
            let restarted = self.key != Some(key);
            if restarted {
                self.toggle = !self.toggle;
            }
            self.key = Some(key);
            actor.torso_animation = animation | if self.toggle { 0x400 } else { 0 };
            restarted
        } else {
            self.key = None;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codol_protocol::game::channel::usercmd::UserCommand;
    use codol_protocol::game::snapshot::Ammo;
    use codol_protocol::game::snapshot::Equipment;
    fn actor() -> EnemySnapshot {
        EnemySnapshot {
            entity_number: 1,
            client_index: 1,
            team: 2,
            model: 4,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: [0.0; 3],
            angles: [0; 3],
            legs_animation: 328,
            torso_animation: 0,
            weapon: 352,
            events: Default::default(),
            alive: true,
        }
    }
    fn player(segmented: bool) -> crate::game::weapons::held::HeldWeapons {
        crate::game::weapons::held::HeldWeapons::new(
            Equipment {
                perks: Default::default(),
                abilities: Default::default(),
                offhands: [None, None],
                offhand_weapon: 0,
                grenade_time: 0,
                move_multiplier: 1.0,
                weapon: 352,
                inventory_weapon: None,
                inventory_override: None,
                alternate_weapon: None,
                alternates: [None, None],
                fire_count: 0,
                queued_burst: false,
                barrel_flags: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
                viewmodel: None,
                firing: None,
                aim: None,
                secondary: None,
                events: None,
                ammo: Some(Ammo {
                    clip_key: 352,
                    reserve_key: 163,
                    clip: 2,
                    reserve: 8,
                }),
            },
            Some(79),
            Some(crate::game::weapons::held::ReloadTiming {
                normal_msec: 500,
                empty_msec: 600,
                add_msec: 300,
                empty_add_msec: 300,
                segmented: segmented.then_some(crate::game::weapons::held::SegmentedReload {
                    start_msec: 100,
                    start_add_msec: 0,
                    end_msec: 100,
                }),
            }),
            None,
        )
    }
    fn command(time: u32, buttons: u32) -> UserCommand {
        UserCommand {
            time,
            buttons,
            weapons: [352, 0, 0],
            ..Default::default()
        }
    }
    #[test]
    fn reload_overlay_tracks_actual_reload_and_keeps_legs_after_ammo_transfer() {
        let mut p = player(false);
        let mut pose = ReloadAnimation::default();
        let mut actor = actor();
        p.apply(&command(1001, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 1);
        p.apply(&command(1100, 16));
        assert!(pose.apply(&mut actor, Some(582), Some(p.equipment())));
        let started = actor.torso_animation;
        assert_eq!(started & 0x3ff, 582);
        for time in [1150, 1200, 1400, 1500] {
            p.apply(&command(time, 0));
            actor.torso_animation = 0;
            assert!(!pose.apply(&mut actor, Some(582), Some(p.equipment())));
            assert_eq!(actor.torso_animation, started);
            assert_eq!(actor.legs_animation, 328);
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 2);
        p.apply(&command(1600, 0));
        actor.torso_animation = 0;
        assert!(!pose.apply(&mut actor, Some(582), Some(p.equipment())));
        assert_eq!(actor.torso_animation, 0);
        p.apply(&command(1700, 1));
        p.apply(&command(1800, 16));
        assert!(pose.apply(&mut actor, Some(582), Some(p.equipment())));
        assert_ne!(actor.torso_animation, started);
    }
    #[test]
    fn shell_reloads_restart_body_on_phase_changes_not_every_snapshot() {
        let mut p = player(true);
        let mut pose = ReloadAnimation::default();
        let mut actor = actor();
        p.apply(&command(1001, 1));
        p.apply(&command(1100, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 0);
        let mut phases = std::collections::BTreeSet::new();
        let mut restarts = 0;
        for time in (1200..=2800).step_by(50) {
            p.apply(&command(time, if time == 1200 { 16 } else { 0 }));
            actor.torso_animation = 0;
            let state = p.equipment().firing.unwrap().state;
            phases.insert(state);
            restarts += usize::from(pose.apply(&mut actor, Some(584), Some(p.equipment())));
            assert_eq!(
                actor.torso_animation & 0x3ff,
                if (8..=12).contains(&state) { 584 } else { 0 }
            );
            assert_eq!(actor.legs_animation, 328);
        }
        assert!(phases.is_superset(&[0, 8, 10, 12].into_iter().collect()));
        assert_eq!(restarts, 3); // start, empty shell, second shell; end continues
        assert_eq!(p.equipment().ammo.unwrap().clip, 2);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 6);
    }
}
