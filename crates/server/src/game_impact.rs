//! Short-lived world events; each ID is handled only once by the client.
use codol_protocol::game_impact::ImpactSnapshot;
use std::collections::VecDeque;

const HOLD_MSEC: u32 = 200;
const MAX_ACTIVE: usize = 32;

#[derive(Default)]
pub(crate) struct ImpactEvents {
    active: VecDeque<(u32, ImpactSnapshot)>,
    next_entity: u16,
}

impl ImpactEvents {
    pub fn push(
        &mut self,
        now: u32,
        weapon: u16,
        hit: crate::game_combat::WorldImpact,
    ) -> Option<ImpactSnapshot> {
        self.push_event(now, weapon, hit, Default::default())
    }
    pub fn explosion(
        &mut self,
        now: u32,
        weapon: u16,
        origin: [f32; 3],
        normal: [f32; 3],
    ) -> Option<ImpactSnapshot> {
        self.push_event(
            now,
            weapon,
            crate::game_combat::WorldImpact {
                origin,
                normal,
                surface: 0,
            },
            codol_protocol::game_impact::ImpactEvent::GrenadeExplosion,
        )
    }
    pub fn smoke(
        &mut self,
        now: u32,
        start_time: u32,
        weapon: u16,
        origin: [f32; 3],
        normal: [f32; 3],
    ) -> Option<ImpactSnapshot> {
        self.push_event(
            now,
            weapon,
            crate::game_combat::WorldImpact {
                origin,
                normal,
                surface: 0,
            },
            codol_protocol::game_impact::ImpactEvent::SmokeExplosion { start_time },
        )
    }
    /// Native EV_CUSTOM_EXPLODE93 without the persistent smoke flag.
    pub fn custom(
        &mut self,
        now: u32,
        start_time: u32,
        weapon: u16,
        origin: [f32; 3],
        normal: [f32; 3],
    ) -> Option<ImpactSnapshot> {
        self.push_event(
            now,
            weapon,
            crate::game_combat::WorldImpact {
                origin,
                normal,
                surface: 0,
            },
            codol_protocol::game_impact::ImpactEvent::CustomExplosion { start_time },
        )
    }
    pub fn tactical(
        &mut self,
        now: u32,
        start_time: u32,
        weapon: u16,
        origin: [f32; 3],
        normal: [f32; 3],
        kind: crate::game_tactical::Kind,
    ) -> Option<ImpactSnapshot> {
        self.push_event(
            now,
            weapon,
            crate::game_combat::WorldImpact {
                origin,
                normal,
                surface: 0,
            },
            match kind {
                crate::game_tactical::Kind::Flash => {
                    codol_protocol::game_impact::ImpactEvent::FlashExplosion
                }
                crate::game_tactical::Kind::Concussion => {
                    codol_protocol::game_impact::ImpactEvent::CustomExplosion { start_time }
                }
                crate::game_tactical::Kind::Gas | crate::game_tactical::Kind::GasNoKick => {
                    return None;
                }
            },
        )
    }
    fn push_event(
        &mut self,
        now: u32,
        weapon: u16,
        hit: crate::game_combat::WorldImpact,
        event: codol_protocol::game_impact::ImpactEvent,
    ) -> Option<ImpactSnapshot> {
        if !(1..900).contains(&weapon)
            || hit.surface > 30
            || hit
                .origin
                .iter()
                .any(|v| !v.is_finite() || !(-32768.0..=32767.0).contains(v))
        {
            return None;
        }
        let normal_byte = codol_protocol::game_impact::normal_byte(hit.normal).ok()?;
        self.expire(now);
        let first = codol_protocol::game_held_weapon::END_ENTITY;
        let entity_number = if !(first..1700).contains(&self.next_entity) {
            first
        } else {
            self.next_entity
        };
        self.next_entity = if entity_number == 1699 {
            first
        } else {
            entity_number + 1
        };
        let impact = ImpactSnapshot {
            event,
            entity_number,
            origin: hit.origin,
            normal_byte,
            surface: hit.surface,
            weapon,
        };
        // Bound snapshot size even when catching up a backlog of commands.
        if self.active.len() == MAX_ACTIVE {
            self.active.pop_front();
        }
        self.active.push_back((now, impact));
        Some(impact)
    }

    fn expire(&mut self, now: u32) {
        self.active
            .retain(|(created, _)| now.wrapping_sub(*created) < HOLD_MSEC);
    }

    pub fn snapshot(&mut self, now: u32) -> Vec<ImpactSnapshot> {
        self.expire(now);
        let mut impacts: Vec<_> = self.active.iter().map(|(_, impact)| *impact).collect();
        impacts.sort_unstable_by_key(|impact| impact.entity_number);
        impacts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> crate::game_combat::WorldImpact {
        crate::game_combat::WorldImpact {
            origin: [90.0, 0.0, 30.0],
            normal: [-1.0, 0.0, 0.0],
            surface: 0,
        }
    }

    #[test]
    fn repeated_shots_have_distinct_ids_and_survive_multiple_frames_then_expire() {
        let mut events = ImpactEvents::default();
        assert_eq!(events.push(1000, 352, hit()).unwrap().entity_number, 54);
        assert_eq!(events.push(1080, 353, hit()).unwrap().entity_number, 55);
        for time in [1080, 1100, 1199] {
            assert_eq!(events.snapshot(time).len(), 2);
        }
        let last = events.snapshot(1200);
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].weapon, 353);
        assert!(events.snapshot(1280).is_empty());
    }

    #[test]
    fn clock_wrap_and_entity_wrap_keep_events_bounded_and_sorted() {
        let mut events = ImpactEvents {
            next_entity: 1699,
            ..ImpactEvents::default()
        };
        let now = u32::MAX - 20;
        events.push(now, 352, hit());
        events.push(now.wrapping_add(10), 352, hit());
        let frame = events.snapshot(now.wrapping_add(199));
        assert_eq!(
            frame.iter().map(|e| e.entity_number).collect::<Vec<_>>(),
            [54, 1699]
        );
        assert!(events.snapshot(now.wrapping_add(210)).is_empty());
        for i in 0..100 {
            events.push(1000 + i, 352, hit());
        }
        assert_eq!(events.snapshot(1100).len(), MAX_ACTIVE);
    }
}
