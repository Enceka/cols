//! Persistent native smoke and a bounded server approximation of its optical emitters.
use codol_protocol::game_smoke::{END_ENTITY, FIRST_ENTITY, SmokeSnapshot};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Definition {
    pub effect: String,
    pub lifetime_msec: u32,
    pub emitters: Vec<Emitter>,
}
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Emitter {
    pub count: [u32; 2],
    pub delay_msec: [u32; 2],
    pub life_msec: [u32; 2],
    /// Radius base/delta and fixed native alpha at each normalized-age keyframe.
    pub samples: Vec<[f32; 3]>,
}
impl Definition {
    pub fn valid(&self) -> bool {
        self.effect == "props/american_smoke_grenade_mp"
            && (1..=120_000).contains(&self.lifetime_msec)
            && !self.emitters.is_empty()
            && self.emitters.iter().all(|e| {
                e.count[0] > 0
                    && e.count.iter().sum::<u32>() <= 64
                    && e.life_msec[0] > 0
                    && e.delay_msec
                        .iter()
                        .chain(e.life_msec.iter())
                        .all(|&t| t <= self.lifetime_msec)
                    && (2..=256).contains(&e.samples.len())
                    && e.samples.iter().all(|s| {
                        s.iter().all(|v| v.is_finite())
                            && s[0] >= 0.0
                            && s[0] + s[1] >= 0.0
                            && (0.0..=1.0).contains(&s[2])
                    })
            })
    }
}
#[derive(Default)]
pub(crate) struct Smoke {
    active: Vec<(SmokeSnapshot, Definition)>,
    sequence: u32,
}
impl Smoke {
    pub fn clear(&mut self) {
        self.active.clear();
    }
    pub fn expire(&mut self, now: u32) {
        self.active.retain(|(s, _)| now <= s.end_time);
    }
    pub fn spawn(&mut self, explosion: &crate::game_grenade::Explosion) -> bool {
        let Some(definition) = explosion.definition.smoke.as_ref() else {
            return false;
        };
        self.expire(explosion.time);
        let Some(entity_number) = (FIRST_ENTITY..END_ENTITY)
            .find(|n| self.active.iter().all(|(s, _)| s.entity_number != *n))
        else {
            return false;
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.active.push((
            SmokeSnapshot {
                entity_number,
                origin: explosion.origin,
                weapon: explosion.definition.index,
                start_time: explosion.time,
                end_time: explosion.time.saturating_add(definition.lifetime_msec),
                present: true,
                generation: self.sequence,
            },
            definition.clone(),
        ));
        true
    }
    pub fn snapshots(&self, now: u32) -> Vec<SmokeSnapshot> {
        let mut states: Vec<_> = self
            .active
            .iter()
            .filter(|(s, _)| now <= s.end_time)
            .map(|(s, _)| *s)
            .collect();
        states.sort_by_key(|s| s.entity_number);
        states
    }
    pub fn visible(&self, now: u32, start: [f32; 3], end: [f32; 3]) -> bool {
        self.transmittance(now, start, end) >= 0.2
    }
    fn transmittance(&self, now: u32, start: [f32; 3], end: [f32; 3]) -> f32 {
        let mut light = 1.0;
        for (state, definition) in &self.active {
            let Some(age) = now.checked_sub(state.start_time) else {
                continue;
            };
            if now > state.end_time {
                continue;
            }
            // Optical particles retain captured count, delay, lifespan and size/alpha
            // curves. Their centers use the detonation origin and range midpoints;
            // this is an AI volume estimate, not the native randomized FX simulation.
            for emitter in &definition.emitters {
                let delay = emitter.delay_msec[0] + emitter.delay_msec[1] / 2;
                let life = emitter.life_msec[0] + emitter.life_msec[1] / 2;
                if age < delay || age - delay >= life {
                    continue;
                }
                let position =
                    (age - delay) as f32 / life as f32 * (emitter.samples.len() - 1) as f32;
                let index = position.floor() as usize;
                let fraction = position - index as f32;
                let sample: [f32; 3] = std::array::from_fn(|i| {
                    emitter.samples[index][i]
                        + fraction * (emitter.samples[index + 1][i] - emitter.samples[index][i])
                });
                let radius = sample[0] + sample[1] * 0.5;
                if segment_intersects(start, end, state.origin, radius) {
                    let count = emitter.count[0] + emitter.count[1] / 2;
                    light *= (1.0 - sample[2]).powi(count as i32);
                }
            }
        }
        light
    }
}
fn segment_intersects(start: [f32; 3], end: [f32; 3], center: [f32; 3], radius: f32) -> bool {
    let direction: [f32; 3] = std::array::from_fn(|i| end[i] - start[i]);
    let length_squared = direction.iter().map(|v| v * v).sum::<f32>();
    let fraction = if length_squared > 0.0 {
        ((0..3)
            .map(|i| (center[i] - start[i]) * direction[i])
            .sum::<f32>()
            / length_squared)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    (0..3)
        .map(|i| (start[i] + direction[i] * fraction - center[i]).powi(2))
        .sum::<f32>()
        < radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;
    fn explosion() -> crate::game_grenade::Explosion {
        crate::game_grenade::Explosion {
            time: 1000,
            origin: [0.0, 0.0, 2.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured("smoke_grenade_mp"),
        }
    }
    #[test]
    fn optical_smoke_fades_before_the_native_canister_and_never_blocks_off_segment_targets() {
        let mut smoke = Smoke::default();
        let explosion = explosion();
        let def = explosion.definition.smoke.as_ref().unwrap();
        assert!(def.valid());
        assert_eq!(def.lifetime_msec, 60001);
        assert_eq!(def.emitters.len(), 2);
        assert!(smoke.spawn(&explosion));
        let ray = ([-500.0, 0.0, 60.0], [500.0, 0.0, 60.0]);
        assert!(smoke.visible(1000, ray.0, ray.1));
        assert!(!smoke.visible(5000, ray.0, ray.1));
        assert!(smoke.visible(5000, [-500.0, 400.0, 60.0], [500.0, 400.0, 60.0]));
        assert!(smoke.visible(5000, [-500.0, 0.0, 60.0], [-400.0, 0.0, 60.0]));
        assert!(!smoke.visible(5000, [0.0, 0.0, 60.0], [0.0, 0.0, 60.0]));
        assert!(smoke.visible(14000, ray.0, ray.1));
        assert_eq!(smoke.snapshots(14000).len(), 1);
        assert_eq!(smoke.snapshots(61001).len(), 1);
        assert!(smoke.snapshots(61002).is_empty());
        smoke.expire(61002);
        assert!(smoke.spawn(&crate::game_grenade::Explosion {
            time: 62000,
            ..explosion
        }));
        let reused = smoke.snapshots(62000)[0];
        assert_eq!(reused.entity_number, FIRST_ENTITY);
        assert_eq!(reused.generation, 2);
        smoke.clear();
        assert!(smoke.snapshots(62000).is_empty());
    }
    #[test]
    fn smoke_slots_are_bounded_and_independent_of_flying_grenades() {
        let mut smoke = Smoke::default();
        for _ in FIRST_ENTITY..END_ENTITY {
            assert!(smoke.spawn(&explosion()));
        }
        assert!(!smoke.spawn(&explosion()));
        let states = smoke.snapshots(1000);
        assert_eq!(states.len(), 32);
        assert!(
            states
                .windows(2)
                .all(|s| s[0].entity_number < s[1].entity_number)
        );
    }
}
