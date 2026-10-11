//! Bounded authoritative history, separate from acknowledged network baselines.
use crate::game_motion::Motion;
use codol_protocol::{
    game_corpse::CorpseSnapshot,
    game_enemy::EnemySnapshot,
    game_hit_feedback::HitKind,
    game_impact::ImpactSnapshot,
    game_obituary::ObituarySnapshot,
    game_snapshot::{Equipment, PlayerVitals},
};
use std::collections::VecDeque;

pub const PRE_DEATH_MSEC: u32 = 3000;
pub const DEATH_VIEW_MSEC: u32 = 1000;
const POST_DEATH_MSEC: u32 = 500;
const HISTORY_MSEC: u32 = 8000;
const MAX_FRAMES: usize = 180;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resources {
    pub ncs_crc: u32,
    pub skip_text: u16,
}
impl Resources {
    pub fn validate(&self, crc: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.ncs_crc == crc && (1..1023).contains(&self.skip_text),
            "invalid map-local replay text resource"
        );
        Ok(())
    }
    pub fn skip_text(&self, kind: Kind) -> Option<u16> {
        (kind == Kind::Death).then_some(self.skip_text)
    }
}

#[derive(Clone)]
pub(crate) struct View {
    pub client: u8,
    pub motion: Motion,
    pub angles: [u16; 3],
    pub equipment: Equipment,
    pub vitals: PlayerVitals,
    pub hit: Option<(u32, HitKind)>,
}
impl View {
    pub fn rebased(mut self, delay: u32) -> Self {
        self.motion.command_time = self.motion.command_time.wrapping_add(delay);
        // Shift absolute clocks, never remaining weapon/animation durations.
        for time in [
            &mut self.motion.jump_time,
            &mut self.motion.view_height_lerp_time,
        ] {
            if *time != 0 {
                *time = time.wrapping_add(delay);
            }
        }
        self.hit = self
            .hit
            .map(|(time, kind)| (time.wrapping_add(delay), kind));
        self
    }
}
#[derive(Clone)]
pub(crate) struct Frame {
    pub time: u32,
    pub views: Vec<View>,
    pub local: Option<EnemySnapshot>,
    pub players: Vec<EnemySnapshot>,
    pub corpses: Vec<CorpseSnapshot>,
    pub impacts: Vec<ImpactSnapshot>,
    pub obituaries: Vec<ObituarySnapshot>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Death,
    Final,
}
#[derive(Clone, Copy)]
struct Playback {
    kind: Kind,
    killer: u8,
    start: u32,
    source_start: u32,
    duration: u32,
    entered: bool,
}
pub(crate) struct Sample {
    pub kind: Kind,
    pub frame: Frame,
    pub view: View,
    pub delay: u32,
    pub epoch: u32,
    pub end_time: u32,
    pub entering: bool,
}
#[derive(Default)]
pub(crate) struct Replay {
    frames: VecDeque<Frame>,
    playback: Option<Playback>,
    pub epoch: u32,
    last_buttons: u32,
    pub last_sound: Option<u32>,
}
impl Replay {
    pub fn record(&mut self, frame: Frame) {
        if self.frames.back().is_some_and(|p| frame.time <= p.time) {
            return;
        }
        while self
            .frames
            .front()
            .is_some_and(|f| frame.time.saturating_sub(f.time) > HISTORY_MSEC)
            || self.frames.len() >= MAX_FRAMES
        {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }
    pub fn start(&mut self, kind: Kind, death: u32, now: u32, killer: u8) -> bool {
        // Match end must never leave an older personal replay in control if
        // the final killer has no usable history.
        if kind == Kind::Final {
            self.finish("final override");
        }
        let Some(latest) = self
            .frames
            .iter()
            .rev()
            .find(|f| f.time <= death)
            .and_then(|f| {
                f.views
                    .iter()
                    .find(|v| v.client == killer && v.vitals.health > 0)
            })
        else {
            return false;
        };
        let mut first_time = death;
        for frame in self.frames.iter().rev().filter(|f| f.time <= death) {
            if !frame.views.iter().any(|v| {
                v.client == killer
                    && v.vitals.health > 0
                    && v.vitals.spawn_count == latest.vitals.spawn_count
            }) {
                break;
            }
            first_time = frame.time;
        }
        let source_start = death.saturating_sub(PRE_DEATH_MSEC).max(first_time);
        if death.saturating_sub(source_start) < 500 || killer >= 12 {
            return false;
        }
        self.finish("superseded");
        self.playback = Some(Playback {
            kind,
            killer,
            start: now + DEATH_VIEW_MSEC,
            source_start,
            duration: death - source_start + POST_DEATH_MSEC,
            entered: false,
        });
        self.last_buttons = 0;
        self.last_sound = None;
        tracing::info!(
            ?kind,
            killer,
            death,
            source_start,
            duration = death - source_start + POST_DEATH_MSEC,
            "killcam scheduled"
        );
        true
    }
    pub fn pending(&self) -> bool {
        self.playback.is_some()
    }
    pub fn kind(&self) -> Option<Kind> {
        self.playback.map(|p| p.kind)
    }
    pub fn advance(&mut self, now: u32) {
        if self
            .playback
            .is_some_and(|p| now >= p.start && now - p.start >= p.duration)
        {
            self.finish("complete");
        }
    }
    pub fn input(&mut self, now: u32, buttons: u32) {
        // 3.3.3.3 F/+activate: measured usercmd transition 0x2000000 ->
        // 0x2000008. 0x20 is not the native activate binding in this build.
        let pressed = buttons & 0x08 != 0 && self.last_buttons & 0x08 == 0;
        self.last_buttons = buttons;
        if pressed
            && self
                .playback
                .is_some_and(|p| p.kind == Kind::Death && now >= p.start)
        {
            self.finish("skip");
        }
    }
    fn finish(&mut self, reason: &str) {
        if let Some(p) = self.playback.take() {
            if p.entered {
                self.epoch = self.epoch.wrapping_add(1);
            }
            tracing::info!(kind=?p.kind,reason,"killcam finished");
        }
    }
    pub fn sample(&mut self, now: u32) -> Option<Sample> {
        self.advance(now);
        let p = self.playback?;
        if now < p.start {
            return None;
        }
        let elapsed = now - p.start;
        if elapsed >= p.duration {
            self.finish("complete");
            return None;
        }
        let source = p.source_start + elapsed;
        let Some(frame) = self.frames.iter().rev().find(|f| f.time <= source).cloned() else {
            self.finish("history unavailable");
            return None;
        };
        let Some(view) = frame.views.iter().find(|v| v.client == p.killer).cloned() else {
            self.finish("killer unavailable");
            return None;
        };
        if source.saturating_sub(frame.time) > 250 {
            self.finish("history gap");
            return None;
        }
        let delay = now - frame.time;
        if delay == 0 || delay > 10_000 {
            self.finish("invalid history time");
            return None;
        }
        // Network baseline epochs distinguish live -> replay -> live, even
        // when none of the first replay packets have been acknowledged yet.
        if !p.entered {
            self.epoch = self.epoch.wrapping_add(1);
            self.playback.as_mut().unwrap().entered = true;
            tracing::info!(kind=?p.kind,killer=p.killer,source=frame.time,delay,"killcam entered");
        }
        Some(Sample {
            kind: p.kind,
            frame,
            view: view.rebased(delay),
            delay,
            epoch: self.epoch,
            end_time: p.start + p.duration,
            entering: !p.entered,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codol_protocol::game_snapshot::{Aim, Firing};
    #[test]
    fn skip_prompt_requires_this_maps_resource_and_a_personal_replay() {
        let r = Resources {
            ncs_crc: 123,
            skip_text: 247,
        };
        r.validate(123).unwrap();
        assert!(r.validate(124).is_err());
        assert_eq!(r.skip_text(Kind::Death), Some(247));
        assert_eq!(r.skip_text(Kind::Final), None);
        for skip_text in [0, 1023, u16::MAX] {
            assert!(
                Resources {
                    ncs_crc: 123,
                    skip_text
                }
                .validate(123)
                .is_err()
            );
        }
    }
    fn frame(time: u32, life: u8) -> Frame {
        let mut motion = Motion::new([time as f32, 20.0, 30.0]);
        motion.command_time = time;
        motion.jump_time = if time > 1500 { 1500 } else { 0 };
        let equipment = Equipment {
            perks: Default::default(),
            abilities: Default::default(),
            move_multiplier: 1.0,
            weapon: 352,
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 1,
            queued_burst: false,
            barrel_flags: 0,
            spread_scale: 0.0,
            needs_rechamber: false,
            ammo: None,
            viewmodel: Some(54),
            firing: Some(Firing {
                animation: 1,
                remaining_msec: 79,
                delay_msec: 0,
                state: 5,
            }),
            aim: Some(Aim {
                fraction: 0.5,
                button: true,
                movement_slow: true,
            }),
            secondary: None,
            events: None,
        };
        Frame {
            time,
            views: vec![View {
                client: 7,
                motion,
                angles: [0, 1234, 0],
                equipment,
                vitals: PlayerVitals {
                    spawn_count: life,
                    ..Default::default()
                },
                hit: Some((time, HitKind::Hit)),
            }],
            local: None,
            players: vec![],
            corpses: vec![],
            impacts: vec![],
            obituaries: vec![],
        }
    }
    fn history() -> Replay {
        let mut replay = Replay::default();
        for time in (1000..=4500).step_by(50) {
            replay.record(frame(time, 1));
        }
        replay
    }
    #[test]
    fn death_view_replays_recorded_killer_with_rebased_clocks_then_returns_live() {
        let mut replay = history();
        assert!(replay.start(Kind::Death, 4000, 4000, 7));
        assert!(replay.sample(4999).is_none());
        assert!(replay.pending());
        let first = replay.sample(5000).unwrap();
        assert_eq!(
            (
                first.frame.time,
                first.view.client,
                first.delay,
                first.epoch
            ),
            (1000, 7, 4000, 1)
        );
        assert_eq!(first.view.motion.origin[0], 1000.0);
        assert!(first.entering);
        assert_eq!(first.view.motion.command_time, 5000);
        assert_eq!(first.view.motion.jump_time, 0);
        assert_eq!(first.view.equipment.firing.unwrap().remaining_msec, 79);
        let later = replay.sample(6500).unwrap();
        assert!(!later.entering);
        assert_eq!(later.view.motion.jump_time, 5500);
        assert_eq!(later.view.hit, Some((6500, HitKind::Hit)));
        assert_eq!(later.view.equipment.aim.unwrap().fraction, 0.5);
        assert_eq!(replay.frames[0].views[0].motion.command_time, 1000);
        assert!(replay.sample(8499).is_some());
        assert!(replay.sample(8500).is_none());
        assert!(!replay.pending());
        assert_eq!(replay.epoch, 2);
    }
    #[test]
    fn skip_is_an_edge_and_final_replay_is_not_cancelled_by_use() {
        let mut replay = history();
        replay.start(Kind::Death, 4000, 4000, 7);
        replay.input(4500, 0x08);
        replay.sample(5000).unwrap();
        replay.input(5050, 0x08);
        assert!(replay.pending());
        replay.input(5100, 0);
        replay.input(5125, 0x20);
        assert!(replay.pending());
        replay.input(5150, 0x08);
        assert!(!replay.pending());
        assert_eq!(replay.epoch, 2);
        replay.start(Kind::Final, 4000, 5500, 7);
        assert!(replay.sample(6500).unwrap().entering);
        replay.input(6600, 0x08);
        assert!(replay.pending());
        replay.advance(10000);
        assert!(!replay.pending());
        assert_eq!(replay.epoch, 4);
    }
    #[test]
    fn history_is_bounded_and_cannot_cross_a_respawn_or_missing_frames() {
        let mut replay = Replay::default();
        for time in (1000..30_000).step_by(50) {
            replay.record(frame(time, 1));
        }
        assert!(replay.frames.len() <= MAX_FRAMES);
        assert!(
            replay.frames.back().unwrap().time - replay.frames.front().unwrap().time
                <= HISTORY_MSEC
        );
        assert!(!replay.start(Kind::Death, 2000, 30000, 7));
        assert!(!replay.start(Kind::Death, 29950, 29950, 6));
        replay.record(frame(30000, 2));
        assert!(!replay.start(Kind::Death, 30000, 30000, 7));
        for time in (30050..=31000).step_by(50) {
            replay.record(frame(time, 2));
        }
        assert!(replay.start(Kind::Death, 31000, 31000, 7));
        assert_eq!(replay.sample(32000).unwrap().frame.time, 30000);
        let mut gaps = Replay::default();
        gaps.record(frame(1000, 1));
        gaps.record(frame(4000, 1));
        assert!(gaps.start(Kind::Death, 4000, 4000, 7));
        assert!(gaps.sample(5500).is_none());
        assert!(!gaps.pending());
    }
    #[test]
    fn unavailable_final_history_cancels_an_older_personal_replay() {
        let mut replay = history();
        assert!(replay.start(Kind::Death, 4000, 4000, 7));
        replay.sample(5000).unwrap();
        assert!(!replay.start(Kind::Final, 5000, 5000, 6));
        assert!(!replay.pending());
        assert_eq!(replay.epoch, 2);
    }
}
