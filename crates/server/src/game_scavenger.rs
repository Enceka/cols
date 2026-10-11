//! Native ITEM bags with reference-script death/stock rules.
//! Expiry/slot bounds and stationary grounded placement are server policies;
//! the current production drop physics and lifetime have not been recovered.
use codol_protocol::game_scavenger::{BagSnapshot, END_ENTITY, FIRST_ENTITY};
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Definition {
    client_sha256: String,
    weapon: u16,
    world_model: String,
    bounds_min: [f32; 3],
    touch_delta_min: [f32; 3],
    touch_delta_max: [f32; 3],
}

static DEFINITION: LazyLock<Definition> = LazyLock::new(|| {
    let d: Definition = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/scavenger-3.3.3.3.json"
    ))
    .expect("captured scavenger definition");
    assert_eq!(d.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(d.world_model, "weapon_scavenger_grenadebag");
    d
});
const LIFETIME: u32 = 30_000;
const MAX_BAGS: usize = 16;
#[derive(Default)]
pub(crate) struct Bags {
    active: Vec<(u32, BagSnapshot)>,
    generations: std::collections::BTreeMap<u16, u32>,
}
impl Bags {
    pub fn clear(&mut self) {
        self.active.clear();
    }
    pub fn expire(&mut self, now: u32) {
        self.active.retain(|(t, _)| now.wrapping_sub(*t) < LIFETIME);
    }
    pub fn drop_for_kill(
        &mut self,
        kill: crate::game_match::Kill,
        collision: &crate::game_motion::Collision,
    ) -> Option<BagSnapshot> {
        if kill.attacker == kill.victim.client_index || kill.time < 2000 {
            return None;
        }
        self.expire(kill.time);
        if self.active.len() >= MAX_BAGS {
            self.active.remove(0);
        }
        let number = (FIRST_ENTITY..END_ENTITY)
            .find(|n| self.active.iter().all(|(_, b)| b.entity_number != *n))?;
        let generation = self.generations.entry(number).or_default();
        *generation = generation.wrapping_add(1);
        let mut origin = kill.victim.origin;
        let start = [origin[0], origin[1], origin[2] + 32.0];
        let end = [origin[0], origin[1], origin[2] - 128.0];
        let ground = collision.movement_surface(start, end);
        if !ground.start_solid && ground.fraction < 1.0 && ground.normal[2] > 0.35 {
            origin[2] = start[2] + (end[2] - start[2]) * ground.fraction;
        }
        origin[2] += -DEFINITION.bounds_min[2] + 0.25;
        // ITEM position codecs publish signed16 integer coordinates. Use the
        // same position for native prediction and authoritative touch gates.
        origin = origin.map(f32::round_ties_even);
        let bag = BagSnapshot {
            entity_number: number,
            origin,
            weapon: DEFINITION.weapon,
            generation: *generation,
        };
        self.active.push((kill.time, bag));
        Some(bag)
    }
    pub fn snapshots(&self) -> Vec<BagSnapshot> {
        let mut bags: Vec<_> = self.active.iter().map(|(_, b)| *b).collect();
        bags.sort_unstable_by_key(|b| b.entity_number);
        bags
    }
    pub fn collect(
        &mut self,
        now: u32,
        origin: [f32; 3],
        alive: bool,
        player: &mut crate::game_player::Player,
    ) -> Option<BagSnapshot> {
        self.expire(now);
        if !alive || !player.can_scavenge() {
            return None;
        }
        let index = self.active.iter().position(|(_, b)| {
            (0..3).all(|i| {
                let delta = origin[i] - b.origin[i];
                (DEFINITION.touch_delta_min[i]..=DEFINITION.touch_delta_max[i]).contains(&delta)
            })
        })?;
        let bag = self.active[index].1;
        if !player.scavenge(bag.weapon) {
            return None;
        }
        self.active.remove(index);
        Some(bag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn kill(time: u32) -> crate::game_match::Kill {
        crate::game_match::Kill {
            falling: false,
            time,
            attacker: 0,
            weapon: 352,
            headshot: false,
            melee: false,
            sequence: 1,
            victim: codol_protocol::game_enemy::EnemySnapshot {
                entity_number: 1,
                client_index: 1,
                team: 2,
                model: 1,
                attachment_models: [0; 10],
                attachment_tags: [0; 10],
                origin: [0.0, 0.0, 4.0],
                angles: [0; 3],
                legs_animation: 1,
                torso_animation: 0,
                weapon: 352,
                events: Default::default(),
                alive: false,
            },
        }
    }
    fn floor() -> crate::game_motion::Collision {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("world.json");
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"captured_contents_mask":crate::game_motion::PLAYER_MASK | crate::game_motion::SHOT_MASK,
            "brushes":[{"center":[0,0,-20],"half":[2000,2000,20],"contents":1,"planes":[]}],"triangles":[]});
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        crate::game_motion::Collision::load(&path).unwrap()
    }
    #[test]
    fn scavenger_drop_is_grounded_bounded_and_does_not_spawn_for_suicide_or_grace() {
        let mut bags = Bags::default();
        let c = floor();
        assert!(bags.drop_for_kill(kill(1999), &c).is_none());
        assert!(
            bags.drop_for_kill(
                crate::game_match::Kill {
                    attacker: 1,
                    ..kill(2000)
                },
                &c
            )
            .is_none()
        );
        let first = bags.drop_for_kill(kill(2000), &c).unwrap();
        assert!((first.origin[2] + DEFINITION.bounds_min[2] - 0.25).abs() <= 0.5);
        for n in 1..=MAX_BAGS {
            bags.drop_for_kill(kill(2000 + n as u32), &c);
        }
        assert_eq!(bags.snapshots().len(), MAX_BAGS);
        assert_eq!(bags.snapshots()[0].generation, 2);
        bags.expire(32000);
        assert_eq!(bags.snapshots().len(), MAX_BAGS);
        bags.expire(32016);
        assert!(bags.snapshots().is_empty());
    }
    #[test]
    fn scavenger_touch_uses_native_aabb_live_health_and_single_consumption() {
        let mut bags = Bags::default();
        let bag = bags.drop_for_kill(kill(2000), &floor()).unwrap();
        let mut p = crate::game_player::tests::scavenger_player(0, 100);
        let origin = [
            bag.origin[0] + 36.0,
            bag.origin[1] - 36.0,
            bag.origin[2] - 88.0,
        ];
        assert!(bags.collect(2001, origin, false, &mut p).is_none());
        assert!(
            bags.collect(2001, [origin[0] + 0.01, origin[1], origin[2]], true, &mut p)
                .is_none()
        );
        assert!(bags.collect(2001, origin, true, &mut p).is_some());
        assert!(bags.collect(2001, origin, true, &mut p).is_none());
        assert_eq!(p.equipment().ammo.unwrap().reserve, 30);
        assert_eq!(p.equipment().events.unwrap().sequence, 1);
        bags.drop_for_kill(kill(2002), &floor());
        bags.clear();
        assert!(bags.snapshots().is_empty());
    }
}
