//! Walkable capsule grid built from the same captured map collision as motion.
use crate::game_motion::Collision;
use std::collections::{BTreeMap, BinaryHeap};
const CELL: f32 = 48.0;
const HEIGHT: f32 = 70.0;
type V3 = [f32; 3];

#[derive(Debug)]
pub(crate) struct Navigation {
    positions: Vec<V3>,
    links: Vec<Vec<usize>>,
    cells: BTreeMap<(i32, i32), Vec<usize>>,
}
impl Navigation {
    pub fn build(starts: impl Iterator<Item = V3>, collision: &Collision) -> Self {
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
                if positions.len() >= 16384 {
                    break;
                }
                let mut z = high;
                // A map cell can have a rooftop and a walkable interior.
                // Keeping only its top floor disconnects enclosed spawns.
                for _ in 0..128 {
                    if z <= low || positions.len() >= 16384 {
                        break;
                    }
                    let start = [x as f32 * CELL, y as f32 * CELL, z];
                    let end = [start[0], start[1], low];
                    let trace = collision.raycast(start, end);
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
                for (dx, dy) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
                    if let Some(others) = cells.get(&(x + dx, y + dy)) {
                        for &other in others {
                            if (positions[index][2] - positions[other][2]).abs() <= CELL * 1.1
                                && Self::clear(positions[index], positions[other], collision)
                            {
                                links[index].push(other);
                                links[other].push(index);
                            }
                        }
                    }
                }
            }
        }
        tracing::info!(
            nodes = positions.len(),
            edges = links.iter().map(Vec::len).sum::<usize>(),
            "built bot walkable navigation"
        );
        Self {
            positions,
            links,
            cells,
        }
    }
    fn clear(a: V3, b: V3, collision: &Collision) -> bool {
        // Lift by the recovered movement step height for stair/slope travel.
        let a = [a[0], a[1], a[2] + 18.0];
        let b = [b[0], b[1], b[2] + 18.0];
        let trace = collision.trace(a, b, HEIGHT);
        !trace.start_solid && !trace.all_solid && trace.fraction >= 1.0
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
    pub fn route(&self, start: V3, goal: V3, collision: &Collision) -> Vec<V3> {
        // A long capsule sweep's broadphase covers much of the map. Use the
        // precomputed graph for distant goals instead of tracing through it.
        if distance(start, goal) <= CELL * 4.0 && Self::clear(start, goal, collision) {
            return vec![goal];
        }
        let (Some(start_index), Some(goal_index)) = (
            self.closest(start, collision),
            self.closest(goal, collision),
        ) else {
            return Vec::new();
        };
        let mut best = vec![f32::INFINITY; self.positions.len()];
        let mut parent = vec![usize::MAX; best.len()];
        let mut visited = vec![false; best.len()];
        let mut open = BinaryHeap::new();
        best[start_index] = 0.0;
        open.push((std::cmp::Reverse(0u32), start_index));
        while let Some((_, index)) = open.pop() {
            if visited[index] {
                continue;
            }
            visited[index] = true;
            if index == goal_index {
                let mut path = vec![goal];
                let mut node = index;
                while node != start_index {
                    path.push(self.positions[node]);
                    node = parent[node];
                }
                path.push(self.positions[start_index]);
                path.reverse();
                return path;
            }
            for &next in &self.links[index] {
                let cost = best[index] + distance(self.positions[index], self.positions[next]);
                if cost < best[next] {
                    best[next] = cost;
                    parent[next] = index;
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
        path: &mut std::collections::VecDeque<V3>,
        collision: &Collision,
    ) -> Option<V3> {
        while path.front().is_some_and(|p| distance(start, *p) < 36.0) {
            path.pop_front();
        }
        for index in (1..path.len().min(4)).rev() {
            if Self::clear(start, path[index], collision) {
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
        let origin = crate::game_bot::grounded_spawn(starts[1][0], &collision).unwrap();
        let goal = crate::game_bot::grounded_spawn(starts[0][0], &collision).unwrap();
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
                pending.extend(&nav.links[node]);
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
        let mut bot = crate::game_bot::BotController::new(origin, [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        let target = crate::game_bot::BotTarget {
            origin: goal,
            eye: [goal[0], goal[1], goal[2] + 60.0],
            alive: true,
        };
        let weapon = crate::game_bot::BotWeapon {
            weapon: 352,
            fire_interval_msec: 79,
            ballistics: crate::game_combat::Ballistics {
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
            let frame = bot.step_navigated(time, Some(target), waypoint, weapon, &collision);
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
        assert!(path.iter().any(|p| p[1].abs() > 220.0));
        for (&a, &b) in std::iter::once(&a).chain(path.iter()).zip(path.iter()) {
            assert!(Navigation::clear(a, b, &collision));
        }
    }
}
