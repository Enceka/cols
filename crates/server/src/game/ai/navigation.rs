//! Capsule walking, directed drops and physics-verified jumps on captured maps.
use crate::game::player::movement::Motion;
use crate::game::world::collision::Collision;
use crate::game::world::collision::WORLD;
use std::collections::{BTreeMap, BinaryHeap};
const CELL: f32 = 48.0;
// Shared tactical respawns extend the graph beyond opening formations. Keep
// the budget bounded while covering both ends of the larger captured maps.
const MAX_NODES: usize = 32768;
const HEIGHT: f32 = 70.0;
type V3 = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Waypoint {
    pub origin: V3,
    /// A jump must approach this departure and finish its landing before the
    /// path can be shortened. Walking shortcuts cannot erase this action.
    pub jump_from: Option<V3>,
    pub runup_msec: u16,
}
impl Waypoint {
    pub fn walk(origin: V3) -> Self {
        Self {
            origin,
            jump_from: None,
            runup_msec: 0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Edge {
    to: usize,
    jump: Option<u16>,
}

pub(crate) fn next_random(random: &mut u64) -> u64 {
    *random = (*random).max(1);
    *random ^= *random << 13;
    *random ^= *random >> 7;
    *random ^= *random << 17;
    *random
}

/// The planner and live controller issue exactly the same released jump edge,
/// forward commands and endpoint braking. Never author a ballistic teleport.
pub(crate) fn jump_command(
    motion: &mut Motion,
    goal: V3,
    dt: u32,
    launch: bool,
) -> codol_protocol::game::channel::usercmd::UserCommand {
    let dx = goal[0] - motion.origin[0];
    let dy = goal[1] - motion.origin[1];
    let moving = dx.hypot(dy) > 6.0;
    if !moving {
        motion.velocity[0] = 0.0;
        motion.velocity[1] = 0.0;
    }
    let yaw = (dy.atan2(dx) * 65536.0 / std::f32::consts::TAU).round() as i32 as u16;
    codol_protocol::game::channel::usercmd::UserCommand {
        time: motion.command_time + dt,
        direction: u8::from(moving),
        angles: [0, yaw, 0],
        buttons: if launch { 0x400 } else { 0 },
        ..Default::default()
    }
}
pub(crate) fn jump_landed(motion: &Motion, goal: V3) -> bool {
    motion.ground_entity == WORLD
        && (motion.origin[0] - goal[0]).hypot(motion.origin[1] - goal[1]) <= 8.0
        && (motion.origin[2] - goal[2]).abs() <= 1.0
}
pub(crate) fn jump_reachable(
    mut motion: Motion,
    goal: V3,
    scale: f32,
    runup_msec: u16,
    collision: &Collision,
) -> bool {
    // Start from rest; build real ground acceleration during the checked run-up.
    motion.velocity[0] = 0.0;
    motion.velocity[1] = 0.0;
    // Initialise support and a released button using the ordinary Pmove path.
    motion.apply(
        &codol_protocol::game::channel::usercmd::UserCommand {
            time: motion.command_time + 1,
            ..Default::default()
        },
        scale,
        false,
        collision,
    );
    if !motion.grounded() || motion.command_time.saturating_sub(motion.jump_time) < 500 {
        return false;
    }
    for step in 0..24 {
        let launch = step * 50 == u32::from(runup_msec);
        let command = jump_command(&mut motion, goal, 50, launch);
        motion.apply(&command, scale, false, collision);
        if launch && motion.jump_time != motion.command_time {
            return false;
        }
        if step * 50 > u32::from(runup_msec) && jump_landed(&motion, goal) {
            return true;
        }
        if motion.origin[2] < goal[2] - 128.0 {
            break;
        }
    }
    false
}

#[derive(Debug)]
pub(crate) struct Navigation {
    positions: Vec<V3>,
    links: Vec<Vec<Edge>>,
    cells: BTreeMap<(i32, i32), Vec<usize>>,
}
impl Navigation {
    pub fn build(starts: impl Iterator<Item = V3>, collision: &Collision) -> Self {
        let planning = collision.navigation_view();
        let collision = &planning;
        let starts: Vec<_> = starts.collect();
        let min_x = starts.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min) - 512.0;
        let max_x = starts
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            + 512.0;
        let min_y = starts.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min) - 512.0;
        let max_y = starts
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            + 512.0;
        let high = starts
            .iter()
            .map(|p| p[2])
            .fold(f32::NEG_INFINITY, f32::max)
            + 512.0;
        let low = starts.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min) - 512.0;
        let mut cells = BTreeMap::new();
        let mut positions = Vec::new();
        for x in (min_x / CELL).floor() as i32..=(max_x / CELL).ceil() as i32 {
            for y in (min_y / CELL).floor() as i32..=(max_y / CELL).ceil() as i32 {
                if positions.len() >= MAX_NODES {
                    break;
                }
                let mut z = high;
                // A map cell can have a rooftop and a walkable interior.
                // Keeping only its top floor disconnects enclosed spawns.
                for _ in 0..128 {
                    if z <= low || positions.len() >= MAX_NODES {
                        break;
                    }
                    let start = [x as f32 * CELL, y as f32 * CELL, z];
                    let end = [start[0], start[1], low];
                    let trace = collision.ground_raycast(start, end);
                    if trace.start_solid || trace.all_solid {
                        z -= 16.0;
                        continue;
                    }
                    if trace.contents == 0 {
                        break;
                    }
                    let floor = z + (low - z) * trace.fraction;
                    let point = [start[0], start[1], floor + 16.0];
                    if trace.normal[2] >= 0.7 {
                        // A point on sloping ground puts the rounded bottom
                        // of the capsule underground. Ground the whole hull.
                        let end = [point[0], point[1], floor - 16.0];
                        let support = collision.trace(point, end, HEIGHT);
                        let position = [
                            point[0],
                            point[1],
                            point[2] + (end[2] - point[2]) * support.fraction,
                        ];
                        let hull = collision.trace(position, position, HEIGHT);
                        if !support.start_solid
                            && !support.all_solid
                            && support.fraction < 1.0
                            && support.normal[2] >= 0.7
                            && !hull.start_solid
                            && !hull.all_solid
                        {
                            cells
                                .entry((x, y))
                                .or_insert_with(Vec::new)
                                .push(positions.len());
                            positions.push(position);
                        }
                    }
                    z = floor - 1.0;
                }
            }
        }
        let mut links = vec![Vec::new(); positions.len()];
        for (&(x, y), indices) in &cells {
            for &index in indices {
                for (dx, dy) in [
                    (1, 0),
                    (0, 1),
                    (1, 1),
                    (1, -1),
                    (2, 0),
                    (0, 2),
                    (2, 1),
                    (2, -1),
                    (1, 2),
                    (1, -2),
                    (3, 0),
                    (0, 3),
                ] {
                    if let Some(others) = cells.get(&(x + dx, y + dy)) {
                        for &other in others {
                            let a = positions[index];
                            let b = positions[other];
                            let local = dx.abs() <= 1 && dy.abs() <= 1;
                            if (a[2] - b[2]).abs() <= CELL * 1.1 && Self::clear(a, b, collision) {
                                if local {
                                    links[index].push(Edge {
                                        to: other,
                                        jump: None,
                                    });
                                    links[other].push(Edge {
                                        to: index,
                                        jump: None,
                                    });
                                }
                            } else {
                                if local && Self::can_drop(a, b, collision) {
                                    links[index].push(Edge {
                                        to: other,
                                        jump: None,
                                    });
                                } else if let Some(runup) = Self::can_jump(a, b, collision) {
                                    links[index].push(Edge {
                                        to: other,
                                        jump: Some(runup),
                                    });
                                }
                                if local && Self::can_drop(b, a, collision) {
                                    links[other].push(Edge {
                                        to: index,
                                        jump: None,
                                    });
                                } else if let Some(runup) = Self::can_jump(b, a, collision) {
                                    links[other].push(Edge {
                                        to: index,
                                        jump: Some(runup),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        tracing::info!(
            nodes = positions.len(),
            edges = links.iter().map(Vec::len).sum::<usize>(),
            jumps = links.iter().flatten().filter(|e| e.jump.is_some()).count(),
            "built bot navigation"
        );
        Self {
            positions,
            links,
            cells,
        }
    }
    /// Candidate roaming goals use actual walkable nodes. Reachability remains
    /// checked by route(); callers budget one graph search per authority tick.
    pub fn roam_goal(&self, origin: V3, occupied: &[V3], random: &mut u64) -> Option<V3> {
        if self.positions.is_empty() {
            return None;
        }
        for _ in 0..12 {
            let p = self.positions[next_random(random) as usize % self.positions.len()];
            if distance(origin, p) > 256.0 && occupied.iter().all(|&q| distance(q, p) > 160.0) {
                return Some(p);
            }
        }
        None
    }
    fn clear(a: V3, b: V3, collision: &Collision) -> bool {
        // Lift by the recovered movement step height for stair/slope travel.
        let raised_a = [a[0], a[1], a[2] + 18.0];
        let raised_b = [b[0], b[1], b[2] + 18.0];
        let trace = collision.trace(raised_a, raised_b, HEIGHT);
        if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
            return false;
        }
        // A clear airborne sweep does not establish a walking route. Sample
        // more closely than the capsule radius, allowing ordinary stairs and
        // slopes but not bridges across deep gaps. Apply this to graph links,
        // entry/exit links and steering shortcuts alike.
        let samples = (distance(a, b) / 12.0).ceil().max(1.0) as usize;
        (0..=samples).all(|i| {
            let t = i as f32 / samples as f32;
            let point: V3 = std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * t);
            let support = collision.trace(
                [point[0], point[1], point[2] + 18.0],
                [point[0], point[1], point[2] - 18.0],
                HEIGHT,
            );
            !support.start_solid
                && !support.all_solid
                && support.fraction < 1.0
                && support.normal[2] >= 0.7
        })
    }
    fn can_jump(a: V3, b: V3, collision: &Collision) -> Option<u16> {
        let horizontal = (a[0] - b[0]).hypot(a[1] - b[1]);
        // Native Pmove's jump rises39 units. Higher negotiation animations
        // require a separate mantle implementation, not a larger impulse.
        if !(24.0..=144.0).contains(&horizontal) || !(-128.0..=39.0).contains(&(b[2] - a[2])) {
            return None;
        }
        // Cheap conservative rejection of walls/ceilings before simulation.
        let trace = collision.trace([a[0], a[1], a[2] + 40.0], [b[0], b[1], b[2] + 40.0], HEIGHT);
        if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
            return None;
        }
        let mut motion = Motion::new(a);
        motion.command_time = 1000;
        [0, 50, 100, 150, 200]
            .into_iter()
            .find(|&runup| jump_reachable(motion.clone(), b, 1.0, runup, collision))
    }
    /// A falling connection has different reachability from its reverse.
    /// Keep it local and bounded; a clear sweep alone cannot admit a gap.
    /// Run the same standing movement, gravity and stopping behavior as Bots,
    /// requiring actual landing on the destination's captured support.
    fn can_drop(a: V3, b: V3, collision: &Collision) -> bool {
        let drop = a[2] - b[2];
        let horizontal = (a[0] - b[0]).hypot(a[1] - b[1]);
        if !(18.0..=128.0).contains(&drop) || !(CELL * 0.5..=CELL * 1.5).contains(&horizontal) {
            return false;
        }
        let mut motion = Motion::new(a);
        motion.apply(
            &codol_protocol::game::channel::usercmd::UserCommand {
                time: 1000,
                ..Default::default()
            },
            1.0,
            false,
            collision,
        );
        for step in 1..=80 {
            let dx = b[0] - motion.origin[0];
            let dy = b[1] - motion.origin[1];
            let moving = dx.hypot(dy) > 16.0;
            if !moving {
                motion.velocity[0] = 0.0;
                motion.velocity[1] = 0.0;
            }
            let yaw = (dy.atan2(dx) * 65536.0 / std::f32::consts::TAU).round() as i32 as u16;
            motion.apply(
                &codol_protocol::game::channel::usercmd::UserCommand {
                    time: 1000 + step * 16,
                    direction: u8::from(moving),
                    angles: [0, yaw, 0],
                    ..Default::default()
                },
                1.0,
                false,
                collision,
            );
            if motion.ground_entity == WORLD
                && (motion.origin[0] - b[0]).hypot(motion.origin[1] - b[1]) <= 18.0
                && (motion.origin[2] - b[2]).abs() <= 1.0
            {
                return true;
            }
            if motion.origin[2] < b[2] - 16.0 {
                return false;
            }
        }
        false
    }
    fn closest(&self, position: V3, collision: &Collision) -> Option<usize> {
        let x = (position[0] / CELL).round() as i32;
        let y = (position[1] / CELL).round() as i32;
        let mut near = Vec::new();
        for dx in -3..=3 {
            for dy in -3..=3 {
                if let Some(indices) = self.cells.get(&(x + dx, y + dy)) {
                    for &i in indices {
                        let d = distance(position, self.positions[i]);
                        if d <= CELL * 3.0 {
                            near.push((i, d));
                        }
                    }
                }
            }
        }
        near.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
        near.into_iter()
            .find(|(i, _)| Self::clear(position, self.positions[*i], collision))
            .map(|(i, _)| i)
    }
    #[cfg(test)]
    pub fn route(&self, start: V3, goal: V3, collision: &Collision) -> Vec<Waypoint> {
        self.route_varied(start, goal, collision, 0, &[])
    }
    pub fn route_varied(
        &self,
        start: V3,
        goal: V3,
        collision: &Collision,
        seed: u64,
        recent: &[V3],
    ) -> Vec<Waypoint> {
        let planning = collision.navigation_view();
        let collision = &planning;
        // A long capsule sweep's broadphase covers much of the map. Use the
        // precomputed graph for distant goals instead of tracing through it.
        if distance(start, goal) <= CELL * 4.0 && Self::clear(start, goal, collision) {
            return vec![Waypoint::walk(goal)];
        }
        let (Some(start_index), Some(goal_index)) = (
            self.closest(start, collision),
            self.closest(goal, collision),
        ) else {
            return Vec::new();
        };
        let mut best = vec![f32::INFINITY; self.positions.len()];
        let mut parent = vec![usize::MAX; best.len()];
        let mut jumps = vec![None; best.len()];
        let mut visited = vec![false; best.len()];
        let mut weights = vec![0.0; best.len()];
        let mut open = BinaryHeap::new();
        best[start_index] = 0.0;
        open.push((std::cmp::Reverse(0u32), start_index));
        while let Some((_, index)) = open.pop() {
            if visited[index] {
                continue;
            }
            visited[index] = true;
            if index == goal_index {
                let mut path = vec![Waypoint::walk(goal)];
                let mut node = index;
                while node != start_index {
                    path.push(Waypoint {
                        origin: self.positions[node],
                        jump_from: jumps[node].map(|_| self.positions[parent[node]]),
                        runup_msec: jumps[node].unwrap_or(0),
                    });
                    node = parent[node];
                }
                path.push(Waypoint::walk(self.positions[start_index]));
                path.reverse();
                return path;
            }
            for edge in &self.links[index] {
                let next = edge.to;
                let p = self.positions[next];
                if weights[next] == 0.0 {
                    // Spatially coherent preferences pick different corridors,
                    // rather than zigzagging between neighbouring grid nodes.
                    let region = ((p[0] / 192.0).floor() as i32 as u64)
                        .wrapping_mul(0x9e3779b97f4a7c15)
                        ^ ((p[1] / 192.0).floor() as i32 as u64).wrapping_mul(0xbf58476d1ce4e5b9)
                        ^ ((p[2] / 96.0).floor() as i32 as u64).wrapping_mul(0x94d049bb133111eb);
                    let mut noise = seed ^ region;
                    let preference = if seed == 0 {
                        0.0
                    } else {
                        (next_random(&mut noise) % 1000) as f32 * 0.001
                    };
                    let revisit = if recent.iter().any(|&q| distance(p, q) < 96.0) {
                        1.0
                    } else {
                        0.0
                    };
                    weights[next] = 1.0 + preference * 0.8 + revisit;
                }
                let cost = best[index]
                    + distance(self.positions[index], p) * weights[next]
                    + if edge.jump.is_some() { 64.0 } else { 0.0 };
                if cost < best[next] {
                    best[next] = cost;
                    parent[next] = index;
                    jumps[next] = edge.jump;
                    let priority =
                        cost + distance(self.positions[next], self.positions[goal_index]);
                    open.push((std::cmp::Reverse((priority * 100.0) as u32), next));
                }
            }
        }
        Vec::new()
    }
    pub fn steering(
        &self,
        start: V3,
        path: &mut std::collections::VecDeque<Waypoint>,
        collision: &Collision,
    ) -> Option<Waypoint> {
        let planning = collision.navigation_view();
        let collision = &planning;
        while path.front().is_some_and(|p| {
            if p.jump_from.is_some() {
                if (start[0] - p.origin[0]).hypot(start[1] - p.origin[1]) > 8.0
                    || (start[2] - p.origin[2]).abs() > 1.0
                {
                    return false;
                }
                let floor = collision.trace(start, [start[0], start[1], start[2] - 0.25], HEIGHT);
                return !floor.start_solid
                    && !floor.all_solid
                    && floor.fraction < 1.0
                    && floor.normal[2] >= 0.7;
            }
            let radius = if path.get(1).is_some_and(|q| q.jump_from.is_some()) {
                6.0
            } else {
                36.0
            };
            distance(start, p.origin) < radius
        }) {
            path.pop_front();
        }
        for index in (1..path.len().min(4)).rev() {
            if path.iter().take(index + 1).any(|p| p.jump_from.is_some()) {
                continue;
            }
            if Self::clear(start, path[index].origin, collision) {
                path.drain(..index);
                break;
            }
        }
        path.front().copied()
    }
}
fn distance(a: V3, b: V3) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn jump_scene(landing: bool, wall: bool, ceiling: bool) -> Collision {
        let mut brushes = vec![serde_json::json!({
            "center":[-1000,0,-100],"half":[1000,500,100],"contents":1,"planes":[]
        })];
        if landing {
            brushes.push(serde_json::json!({
                "center":[1064,0,-100],"half":[1000,500,100],"contents":1,"planes":[]
            }));
        }
        if wall {
            brushes.push(serde_json::json!({
                "center":[32,0,100],"half":[4,500,100],"contents":1,"planes":[]
            }));
        }
        if ceiling {
            brushes.push(serde_json::json!({
                "center":[0,0,100],"half":[500,500,5],"contents":1,"planes":[]
            }));
        }
        let data = serde_json::json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }
    #[test]
    fn jumping_crosses_a_gap_and_requires_clearance_and_supported_landing() {
        let a = [-48.0, 0.0, 0.03125];
        let b = [96.0, 0.0, 0.03125];
        let collision = jump_scene(true, false, false);
        assert!(!Navigation::clear(a, b, &collision));
        assert!(Navigation::can_jump(a, b, &collision).is_some());
        assert!(Navigation::can_jump(a, b, &jump_scene(false, false, false)).is_none());
        assert!(Navigation::can_jump(a, b, &jump_scene(true, true, false)).is_none());
        assert!(Navigation::can_jump(a, b, &jump_scene(true, false, true)).is_none());
        assert!(Navigation::can_jump(a, [96.0, 0.0, 80.0], &collision).is_none());
        assert!(Navigation::can_jump(a, [240.0, 0.0, 0.03125], &collision).is_none());
        let nav = Navigation::build([a, b].into_iter(), &collision);
        let path = nav.route(a, b, &collision);
        assert!(path.iter().any(|p| p.jump_from.is_some()));
        let mut path = path.into();
        let waypoint = nav.steering(a, &mut path, &collision).unwrap();
        assert!(waypoint.jump_from.is_some(), "must retain the jump action");
        let mut bot = crate::game::ai::controller::BotController::new(a, [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        let weapon = crate::game::ai::controller::BotWeapon {
            weapon: 352,
            fire_interval_msec: 79,
            ballistics: crate::game::combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let mut position = a;
        let mut airborne = false;
        let mut landed = false;
        for now in (1000..5000).step_by(50) {
            let waypoint = nav.steering(position, &mut path, &collision);
            let frame = bot.step_route(now, None, waypoint, weapon, &collision);
            position = frame.motion.origin;
            airborne |= frame.motion.flags & 0x2000 != 0 && position[2] > 10.0;
            if airborne && jump_landed(&frame.motion, b) {
                landed = true;
                break;
            }
        }
        assert!(
            airborne && landed,
            "jump failed: position={position:?}, path={path:?}"
        );
        assert_eq!(bot.jump_count(), 1);
    }
    #[test]
    fn jump_approaches_a_low_platform_and_keeps_steering_through_clock_wrap_and_enemy_acquisition()
    {
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[
            {"center":[-1000,0,-100],"half":[1032,500,100],"contents":1,"planes":[]},
            {"center":[1032,0,-70],"half":[1000,500,100],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        let a = [0.0, 0.0, 0.03125];
        let b = [96.0, 0.0, 30.03125];
        let runup =
            Navigation::can_jump(a, b, &collision).expect("low platform should be jumpable");
        let waypoint = Waypoint {
            origin: b,
            jump_from: Some(a),
            runup_msec: runup,
        };
        let weapon = crate::game::ai::controller::BotWeapon {
            weapon: 352,
            fire_interval_msec: 79,
            ballistics: crate::game::combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let mut bot = crate::game::ai::controller::BotController::new(a, [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        let start = u32::MAX - 800;
        let mut landed = false;
        let mut airborne = false;
        for step in 0..80 {
            let now = start.wrapping_add(step * 50);
            let enemy = airborne.then_some(crate::game::ai::controller::BotTarget {
                origin: a,
                eye: [a[0], a[1], a[2] + 60.0],
                alive: true,
            });
            let frame = bot.step_route(now, enemy, Some(waypoint), weapon, &collision);
            airborne |= frame.motion.flags & 0x2000 != 0;
            let duplicate = bot.step_route(now, enemy, Some(waypoint), weapon, &collision);
            assert_eq!(duplicate.motion.origin, frame.motion.origin);
            assert!(duplicate.landings.is_empty());
            if airborne && jump_landed(&frame.motion, b) {
                landed = true;
                break;
            }
        }
        assert!(
            landed,
            "jump should complete despite an enemy appearing and authority clock wrapping"
        );
        assert_eq!(bot.jump_count(), 1);
        let now = start.wrapping_add(4000);
        assert!(bot.kill(now));
        assert!(!bot.traversing());
        let dead = bot.step_route(
            now.wrapping_add(50),
            None,
            Some(waypoint),
            weapon,
            &collision,
        );
        assert_eq!(dead.phase, crate::game::ai::controller::BotPhase::Dead);
        bot.respawn(now.wrapping_add(100), a, [0; 3]).unwrap();
        assert!(!bot.traversing());
        let fresh = bot.step_route(now.wrapping_add(150), None, None, weapon, &collision);
        assert_eq!(fresh.motion.jump_time, 0);
    }
    #[test]
    fn regional_route_preferences_are_repeatable_and_use_both_corridors() {
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[
            {"center":[0,0,-10],"half":[2000,2000,10],"contents":1,"planes":[]},
            {"center":[0,0,100],"half":[20,200,100],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        let a = [-300.0, 0.0, 0.125];
        let b = [300.0, 0.0, 0.125];
        let nav = Navigation::build([a, b].into_iter(), &collision);
        let mut sides = std::collections::BTreeSet::new();
        for seed in 1..=32 {
            let route = nav.route_varied(a, b, &collision, seed, &[]);
            assert_eq!(route, nav.route_varied(a, b, &collision, seed, &[]));
            let crossing = route
                .iter()
                .min_by(|p, q| p.origin[0].abs().total_cmp(&q.origin[0].abs()))
                .unwrap();
            sides.insert(crossing.origin[1] > 0.0);
            let length: f32 = std::iter::once(a)
                .chain(route.iter().map(|p| p.origin))
                .zip(route.iter().map(|p| p.origin))
                .map(|(p, q)| distance(p, q))
                .sum();
            assert!(length < 1300.0, "excessive detour: {length}");
        }
        assert_eq!(
            sides.len(),
            2,
            "different seeds should use both sides of the wall"
        );
        let first = nav.route(a, b, &collision);
        let recent: Vec<_> = first.iter().map(|p| p.origin).collect();
        let alternate = nav.route_varied(a, b, &collision, 0, &recent);
        let side =
            |r: &[Waypoint]| r.iter().find(|p| p.origin[0].abs() < 24.0).unwrap().origin[1] > 0.0;
        assert_ne!(
            side(&first),
            side(&alternate),
            "recent corridor should be avoided"
        );
        let mut zero = 0;
        assert_ne!(next_random(&mut zero), 0);
    }
    #[test]
    fn bounded_drop_routes_are_one_way_and_require_real_landing_support() {
        let scene = |lower: bool, wall: bool| {
            let mut brushes = vec![serde_json::json!({
                "center":[-1000,0,-100],"half":[1000,1000,100],"contents":1,"planes":[]
            })];
            if lower {
                brushes.push(serde_json::json!({
                    "center":[1000,0,-148],"half":[1000,1000,100],"contents":1,"planes":[]
                }));
            }
            if wall {
                brushes.push(serde_json::json!({
                    "center":[24,0,20],"half":[1,1000,100],"contents":1,"planes":[]
                }));
            }
            let export = serde_json::json!({
                "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
                "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]
            });
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
            Collision::load(file.path()).unwrap()
        };
        let a = [0.0, 0.0, 0.03125];
        let b = [48.0, 0.0, -47.96875];
        let collision = scene(true, false);
        assert!(!Navigation::clear(a, b, &collision));
        assert!(Navigation::can_drop(a, b, &collision));
        assert!(!Navigation::can_drop(b, a, &collision));
        let nav = Navigation::build([a, b].into_iter(), &collision);
        assert!(!nav.route(a, b, &collision).is_empty());
        assert!(nav.route(b, a, &collision).is_empty());
        assert!(!Navigation::can_drop(a, b, &scene(false, false)));
        assert!(!Navigation::can_drop(a, b, &scene(true, true)));
        assert!(!Navigation::can_drop(a, [48.0, 0.0, -256.0], &collision));
        assert!(!Navigation::can_drop(a, [480.0, 0.0, -48.0], &collision));
    }
    #[test]
    fn sight_clip_volumes_do_not_remove_player_walkable_ground() {
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[
                {"center":[0,0,-10],"half":[2000,2000,10],"contents":1,"planes":[]},
                {"center":[0,0,0],"half":[2000,2000,48],"contents":0x08000800,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        assert!(
            (collision
                .raycast([0.0, 0.0, 200.0], [0.0, 0.0, -200.0])
                .fraction
                - 0.38)
                .abs()
                < 0.001
        );
        let start = [-300.0, 0.0, 0.125];
        let end = [300.0, 0.0, 0.125];
        let nav = Navigation::build([start, end].into_iter(), &collision);
        assert!(!nav.route(start, end, &collision).is_empty());
        assert!(nav.positions.iter().all(|p| p[2] < 1.0));
    }
    #[test]
    #[ignore = "uses locally captured Bog collision to verify both team spawn areas"]
    fn captured_bog_ground_connects_both_teams() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("tdm-bog-state.json")).unwrap())
                .unwrap();
        let config: crate::game::modes::tdm::MatchConfig =
            serde_json::from_value(state["match_config"].clone()).unwrap();
        let collision = Collision::load(std::path::Path::new(
            state["collision_file"].as_str().unwrap(),
        ))
        .unwrap();
        let nav = Navigation::build(config.spawns.iter().flatten().copied(), &collision);
        for &point in config.spawns.iter().flatten() {
            let point = crate::game::ai::controller::grounded_spawn(point, &collision).unwrap();
            assert!(
                nav.closest(point, &collision).is_some(),
                "unlinked spawn {point:?}"
            );
        }
        let a =
            crate::game::ai::controller::grounded_spawn(config.spawns[0][0], &collision).unwrap();
        let b =
            crate::game::ai::controller::grounded_spawn(config.spawns[1][0], &collision).unwrap();
        let path = nav.route(a, b, &collision);
        assert!(
            !path.is_empty(),
            "opposing spawn areas must connect across player-walkable terrain"
        );
        for (start, end) in std::iter::once(a)
            .chain(path.iter().map(|p| p.origin))
            .zip(path.iter().map(|p| p.origin))
        {
            assert!(Navigation::clear(start, end, &collision));
        }
        std::fs::write(root.join("bog-navigation-captured-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "map":state["map"],"ncs_crc":state["ncs_crc"],"source":state["collision_file"],
            "nodes":nav.positions.len(),"route_nodes":path.len(),"linked_spawns":config.spawns.iter().map(Vec::len).sum::<usize>(),
            "server_geometry_verified":true,"native_visual_verified":false
        })).unwrap()).unwrap();
    }
    #[test]
    fn walking_routes_and_shortcuts_keep_support_over_a_deep_gap() {
        // The old elevated sweep saw no wall and cut straight across this
        // trench. Bots then fell into a pit they cannot walk back out of.
        let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[
                {"center":[0,0,-210],"half":[600,600,10],"contents":1,"planes":[]},
                {"center":[-324,0,-100],"half":[276,600,100],"contents":1,"planes":[]},
                {"center":[324,0,-100],"half":[276,600,100],"contents":1,"planes":[]},
                {"center":[0,-350,-100],"half":[48,250,100],"contents":1,"planes":[]},
                {"center":[0,350,-100],"half":[48,250,100],"contents":1,"planes":[]}],"triangles":[]});
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = Collision::load(f.path()).unwrap();
        let a = [-96.0, 0.0, 0.03125];
        let b = [96.0, 0.0, 0.03125];
        assert!(!Navigation::clear(a, b, &collision));
        let nav = Navigation::build([a, b].into_iter(), &collision);
        let path = nav.route(a, b, &collision);
        assert!(path.len() > 2);
        assert!(path.iter().all(|p| p.origin[2] > -1.0));
        assert!(
            path.iter()
                .any(|p| p.origin[1].abs() >= 96.0 || p.jump_from.is_some())
        );
        let mut queue = path.into();
        let steering = nav.steering(a, &mut queue, &collision).unwrap();
        assert!(Navigation::clear(a, steering.origin, &collision) || steering.jump_from.is_some());
        assert_ne!(steering.origin, b);
    }
    #[test]
    fn walking_support_checks_preserve_stairs_and_sloping_terrain() {
        let worlds = [
            serde_json::json!({"brushes":[
                {"center":[0,0,-10],"half":[1000,1000,10],"contents":1,"planes":[]},
                {"center":[0,0,6],"half":[48,500,6],"contents":1,"planes":[]},
                {"center":[96,0,12],"half":[48,500,12],"contents":1,"planes":[]}],"triangles":[]}),
            serde_json::json!({"brushes":[],"triangles":[
                {"vertices":[[-1000,-1000,-250],[1000,-1000,250],[1000,1000,250]],"contents":1,"surface_flags":0},
                {"vertices":[[-1000,-1000,-250],[1000,1000,250],[-1000,1000,-250]],"contents":1,"surface_flags":0}]}),
        ];
        for mut export in worlds {
            export["schema_version"] = 1.into();
            export["read_only"] = true.into();
            export["client_sha256"] = codol_protocol::CLIENT_SHA256.into();
            export["map"] = "maps/mp/test.d3dbsp".into();
            export["ncs_crc"] = 0.into();
            let f = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(f.path(), serde_json::to_vec(&export).unwrap()).unwrap();
            let collision = Collision::load(f.path()).unwrap();
            let a = crate::game::ai::controller::grounded_spawn([-96.0, 0.0, 100.0], &collision)
                .unwrap();
            let b = crate::game::ai::controller::grounded_spawn([96.0, 0.0, 100.0], &collision)
                .unwrap();
            assert!(Navigation::clear(a, b, &collision), "{a:?} -> {b:?}");
            assert!(Navigation::clear(b, a, &collision), "{b:?} -> {a:?}");
        }
    }
    #[test]
    #[ignore = "requires locally captured Highrise collision"]
    fn captured_highrise_jumps_land_with_the_live_bot_controller() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("tdm-highrise-state.json")).unwrap())
                .unwrap();
        let starts: [Vec<V3>; 2] =
            serde_json::from_value(state["match_config"]["spawns"].clone()).unwrap();
        let collision = Collision::load(std::path::Path::new(
            state["collision_file"].as_str().unwrap(),
        ))
        .unwrap();
        let before = std::time::Instant::now();
        let nav = Navigation::build(starts.iter().flatten().copied(), &collision);
        let build_msec = before.elapsed().as_millis();
        let jump_edges = nav
            .links
            .iter()
            .flatten()
            .filter(|e| e.jump.is_some())
            .count();
        let weapon = crate::game::ai::controller::BotWeapon {
            weapon: 352,
            fire_interval_msec: 79,
            ballistics: crate::game::combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let mut landed = Vec::new();
        for (index, edges) in nav.links.iter().enumerate() {
            for edge in edges {
                let Some(runup) = edge.jump else {
                    continue;
                };
                let a = nav.positions[index];
                let b = nav.positions[edge.to];
                let mut motion = Motion::new(a);
                motion.command_time = 1000;
                if !jump_reachable(motion, b, 1.0, runup, &collision) {
                    continue;
                }
                let mut bot =
                    crate::game::ai::controller::BotController::new(a, [0; 3], 190.0).unwrap();
                bot.set_match_engagement();
                let waypoint = Waypoint {
                    origin: b,
                    jump_from: Some(a),
                    runup_msec: runup,
                };
                let mut end = a;
                let mut peak = a[2];
                let mut success = false;
                for now in (1000..4000).step_by(50) {
                    let frame = bot.step_route(now, None, Some(waypoint), weapon, &collision);
                    end = frame.motion.origin;
                    peak = peak.max(end[2]);
                    if bot.jump_count() > 0 && jump_landed(&frame.motion, b) {
                        success = true;
                        break;
                    }
                }
                assert!(
                    success,
                    "checked Highrise jump failed: {a:?} -> {b:?}, runup={runup}, end={end:?}"
                );
                landed.push(serde_json::json!({"departure":a,"goal":b,"landed":end,"runup_msec":runup,"peak_z":peak,"jumps":bot.jump_count()}));
                if landed.len() >= 12 {
                    break;
                }
            }
            if landed.len() >= 12 {
                break;
            }
        }
        assert!(
            !landed.is_empty(),
            "Highrise should have physically executable jumps"
        );
        let output = root.join("bot-navigation-20261013");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("highrise-jumps.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "map":"mp_highrise","ncs_crc":state["ncs_crc"],"native_client_used":false,
            "nodes":nav.positions.len(),"jump_edges":jump_edges,"build_msec":build_msec,"landed":landed
        })).unwrap()).unwrap();
        eprintln!(
            "Highrise: nodes={} jumps={jump_edges} landed={} build_ms={build_msec}",
            nav.positions.len(),
            landed.len()
        );
    }
    #[test]
    #[ignore = "requires local three-map collision preparations"]
    fn captured_three_maps_keep_opposing_spawns_connected() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        for name in [
            "tdm-afghan-hud-state",
            "tdm-killhouse-hud-state",
            "tdm-crash-tropical-state",
        ] {
            let data: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap())
                    .unwrap();
            let starts: [Vec<V3>; 2] =
                serde_json::from_value(data["match_config"]["spawns"].clone()).unwrap();
            let collision =
                Collision::load(&root.join(data["collision_file"].as_str().unwrap())).unwrap();
            let before = std::time::Instant::now();
            let nav = Navigation::build(starts.iter().flatten().copied(), &collision);
            let build = before.elapsed();
            for (team, spawns) in starts.iter().enumerate() {
                let goal =
                    crate::game::ai::controller::grounded_spawn(starts[1 - team][0], &collision)
                        .unwrap();
                for &spawn in spawns {
                    let start =
                        crate::game::ai::controller::grounded_spawn(spawn, &collision).unwrap();
                    assert!(
                        !nav.route(start, goal, &collision).is_empty(),
                        "{name} team {team}: {start:?} -> {goal:?}"
                    );
                }
            }
            eprintln!(
                "{name}: nodes={} edges={} spawns={} build_ms={}",
                nav.positions.len(),
                nav.links.iter().map(Vec::len).sum::<usize>(),
                starts.iter().map(Vec::len).sum::<usize>(),
                build.as_millis()
            );
        }
    }
    #[test]
    #[ignore = "requires captured local map collision"]
    fn captured_afghan_teams_have_a_connected_route() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research-output/tdm-state.json");
        let state: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        let starts: [Vec<V3>; 2] =
            serde_json::from_value(state["match_config"]["spawns"].clone()).unwrap();
        let collision = Collision::load(std::path::Path::new(
            state["collision_file"].as_str().unwrap(),
        ))
        .unwrap();
        let nav = Navigation::build(starts.iter().flatten().copied(), &collision);
        let origin = crate::game::ai::controller::grounded_spawn(starts[1][0], &collision).unwrap();
        let goal = crate::game::ai::controller::grounded_spawn(starts[0][0], &collision).unwrap();
        let path = nav.route(origin, goal, &collision);
        let start_node = nav.closest(origin, &collision);
        let goal_node = nav.closest(goal, &collision);
        let mut reached = std::collections::BTreeSet::new();
        let mut pending = Vec::new();
        if let Some(node) = start_node {
            pending.push(node);
        }
        while let Some(node) = pending.pop() {
            if reached.insert(node) {
                pending.extend(nav.links[node].iter().map(|e| e.to));
            }
        }
        println!(
            "start {origin:?} node {start_node:?}; goal {goal:?} node {goal_node:?}; component {} / {} nodes",
            reached.len(),
            nav.positions.len()
        );
        let debug = serde_json::json!({"start":origin,"goal":goal,"start_node":start_node,"goal_node":goal_node,"positions":nav.positions,"component":reached});
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../research-output/nav-debug.json"),
            serde_json::to_vec(&debug).unwrap(),
        )
        .unwrap();
        assert!(
            !path.is_empty(),
            "no route between actual team starts; {} nodes",
            nav.positions.len()
        );
        let mut route: std::collections::VecDeque<_> = path.into();
        let mut bot =
            crate::game::ai::controller::BotController::new(origin, [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        let target = crate::game::ai::controller::BotTarget {
            origin: goal,
            eye: [goal[0], goal[1], goal[2] + 60.0],
            alive: true,
        };
        let weapon = crate::game::ai::controller::BotWeapon {
            weapon: 352,
            fire_interval_msec: 79,
            ballistics: crate::game::combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let mut position = origin;
        let mut fired = false;
        for time in (1000..121000).step_by(50) {
            let waypoint = nav.steering(position, &mut route, &collision);
            let frame = bot.step_route(time, Some(target), waypoint, weapon, &collision);
            position = frame.motion.origin;
            if frame.shot.is_some() {
                fired = true;
                break;
            }
        }
        assert!(
            fired,
            "actual capsule failed to reach engagement range: {position:?}, {} route nodes remain",
            route.len()
        );
    }
    #[test]
    fn route_goes_around_a_wall_with_capsule_clearance() {
        let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[
                {"center":[0,0,-10],"half":[2000,2000,10],"contents":1,"planes":[]},
                {"center":[0,0,100],"half":[20,200,100],"contents":1,"planes":[]}],"triangles":[]});
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = Collision::load(f.path()).unwrap();
        let a = [-300.0, 0.0, 0.125];
        let b = [300.0, 0.0, 0.125];
        let nav = Navigation::build([a, b].into_iter(), &collision);
        let path = nav.route(a, b, &collision);
        assert!(path.len() > 2);
        assert!(path.iter().any(|p| p.origin[1].abs() > 220.0));
        for (a, b) in std::iter::once(a)
            .chain(path.iter().map(|p| p.origin))
            .zip(path.iter().map(|p| p.origin))
        {
            assert!(Navigation::clear(a, b, &collision));
        }
    }
}
