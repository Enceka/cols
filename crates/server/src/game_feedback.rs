//! Feedback is driven by committed damage, independently of the kill feed.
use anyhow::{Result, ensure};
use codol_protocol::game_hit_feedback::{HitFeedbackSnapshot, HitKind};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resources {
    pub ncs_crc: u32,
    pub hit_material: u16,
    pub kill_material: u16,
    #[serde(default)]
    pub death_material: Option<u16>,
    #[serde(default)]
    pub scavenger_material: Option<u16>,
    pub sound_index: u16,
    pub register_sound: bool,
}
impl Resources {
    pub fn validate(&self, ncs_crc: u32) -> Result<()> {
        ensure!(
            self.ncs_crc == ncs_crc,
            "feedback resources belong to another NCS set"
        );
        ensure!(
            (1..1024).contains(&self.hit_material)
                && (1..1024).contains(&self.kill_material)
                && self.hit_material != self.kill_material,
            "invalid feedback material slots"
        );
        ensure!(
            (1..512).contains(&self.sound_index),
            "invalid feedback sound slot"
        );
        ensure!(
            self.death_material.is_none_or(|m| (1..1024).contains(&m)
                && m != self.hit_material
                && m != self.kill_material),
            "invalid death waypoint material"
        );
        ensure!(
            self.scavenger_material
                .is_none_or(|m| (1..1024).contains(&m)
                    && m != self.hit_material
                    && m != self.kill_material),
            "invalid scavenger material slot"
        );
        Ok(())
    }
    pub fn sound_configstring(&self) -> Option<(u16, String)> {
        // 3.3.3.3 category6: base3107, 512 slots; never use 2.6's base0xf32.
        self.register_sound
            .then(|| (3107 + self.sound_index, "MP_hit_alert".into()))
    }
    pub fn sound_command(&self) -> String {
        // 0x57b500 requires exactly one argument, the relative sound slot.
        format!("p {}", self.sound_index)
    }
}

#[derive(Default)]
pub(crate) struct Feedback {
    last: Option<(u32, HitKind)>,
    pending_sounds: u32,
}
impl Feedback {
    pub fn event(&self) -> Option<(u32, HitKind)> {
        self.last
    }
    pub fn record(&mut self, time: u32, kind: HitKind) {
        self.last = Some((time, kind));
        if kind != HitKind::Scavenger {
            self.pending_sounds = self.pending_sounds.saturating_add(1);
        }
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn take_sounds(&mut self) -> u32 {
        std::mem::take(&mut self.pending_sounds)
    }
    pub fn snapshot(&self, now: u32, resources: &Resources) -> HitFeedbackSnapshot {
        let Some((event_time, kind)) = self.last else {
            return HitFeedbackSnapshot::default();
        };
        HitFeedbackSnapshot {
            material: match kind {
                HitKind::Hit => resources.hit_material,
                HitKind::Kill => resources.kill_material,
                HitKind::Scavenger => resources.scavenger_material.unwrap_or(0),
            },
            event_time,
            visible: now.wrapping_sub(event_time) < kind.duration_msec()
                && (kind != HitKind::Scavenger || resources.scavenger_material.is_some()),
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resources() -> Resources {
        Resources {
            ncs_crc: 123,
            hit_material: 125,
            kill_material: 144,
            death_material: Some(169),
            scavenger_material: Some(161),
            sound_index: 9,
            register_sound: true,
        }
    }
    #[test]
    fn feedback_replaces_color_fades_from_hit_time_and_drains_sound_once() {
        let r = resources();
        let mut feedback = Feedback::default();
        feedback.record(1000, HitKind::Kill);
        assert_eq!(feedback.snapshot(1000, &r).material, 144);
        assert_eq!(feedback.snapshot(1599, &r).kind.rgb(), [255, 41, 20]);
        assert!(!feedback.snapshot(1600, &r).visible);
        assert_eq!(feedback.take_sounds(), 1);
        assert_eq!(feedback.take_sounds(), 0);
        feedback.record(1700, HitKind::Kill);
        feedback.record(1800, HitKind::Hit); // Another target: no invented red lock.
        let snapshot = feedback.snapshot(1800, &r);
        assert_eq!(snapshot.material, 125);
        assert_eq!(snapshot.kind.rgb(), [255; 3]);
        assert!(feedback.snapshot(2799, &r).visible);
        assert!(!feedback.snapshot(2800, &r).visible);
        assert_eq!(feedback.take_sounds(), 2);
        feedback.record(u32::MAX - 300, HitKind::Kill);
        assert!(feedback.snapshot(100, &r).visible);
        assert!(!feedback.snapshot(300, &r).visible);
        feedback.clear();
        assert!(!feedback.snapshot(100, &r).visible);
        assert_eq!(feedback.take_sounds(), 0);
    }
    #[test]
    fn resources_bind_map_crc_and_keep_sound_slot_relative() {
        let mut r = resources();
        r.validate(123).unwrap();
        assert!(r.validate(124).is_err());
        assert_eq!(r.sound_configstring(), Some((3116, "MP_hit_alert".into())));
        assert_eq!(r.sound_command(), "p 9");
        r.register_sound = false;
        assert!(r.sound_configstring().is_none());
        for invalid in [0, 512, u16::MAX] {
            r.sound_index = invalid;
            assert!(r.validate(123).is_err());
        }
    }
    #[test]
    fn scavenger_hud_fades_without_queuing_a_second_pickup_sound() {
        let mut feedback = Feedback::default();
        feedback.record(1000, HitKind::Scavenger);
        assert_eq!(feedback.take_sounds(), 0);
        let r = resources();
        let snapshot = feedback.snapshot(1000, &r);
        assert_eq!(snapshot.material, 161);
        assert!(snapshot.visible);
        assert!(!feedback.snapshot(3500, &r).visible);
        let mut absent = r;
        absent.scavenger_material = None;
        assert!(!feedback.snapshot(1000, &absent).visible);
    }
}
