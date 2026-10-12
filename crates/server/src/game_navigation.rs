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
    /// Candidate roaming goals use actual walkable nodes. Reachability remains
    /// checked by route(); callers budget one graph search per authority tick.
    pub fn roam_goal(&self, origin: V3, occupied: &[V3], random: &mut u64) -> Option<V3> {
        if self.positions.is_empty() {
            return None;
        }
        for _ in 0..12 {
            *random ^= *random << 13;
            *random ^= *random >> 7;
            *random ^= *random << 17;
            let p = self.positions[*random as usize % self.positions.len()];
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
        let config: crate::game_match::MatchConfig =
            serde_json::from_value(state["match_config"].clone()).unwrap();
        let collision = Collision::load(std::path::Path::new(
            state["collision_file"].as_str().unwrap(),
        ))
        .unwrap();
        let nav = Navigation::build(config.spawns.iter().flatten().copied(), &collision);
        for &point in config.spawns.iter().flatten() {
            let point = crate::game_bot::grounded_spawn(point, &collision).unwrap();
            assert!(
                nav.closest(point, &collision).is_some(),
                "unlinked spawn {point:?}"
            );
        }
        let a = crate::game_bot::grounded_spawn(config.spawns[0][0], &collision).unwrap();
        let b = crate::game_bot::grounded_spawn(config.spawns[1][0], &collision).unwrap();
        let path = nav.route(a, b, &collision);
        assert!(
            !path.is_empty(),
            "opposing spawn areas must connect across player-walkable terrain"
        );
        for (&start, &end) in std::iter::once(&a).chain(path.iter()).zip(path.iter()) {
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
        assert!(path.iter().all(|p| p[2] > -1.0));
        assert!(path.iter().any(|p| p[1].abs() >= 96.0));
        let mut queue = path.into();
        let steering = nav.steering(a, &mut queue, &collision).unwrap();
        assert!(Navigation::clear(a, steering, &collision));
        assert_ne!(steering, b);
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
            let a = crate::game_bot::grounded_spawn([-96.0, 0.0, 100.0], &collision).unwrap();
            let b = crate::game_bot::grounded_spawn([96.0, 0.0, 100.0], &collision).unwrap();
            assert!(Navigation::clear(a, b, &collision), "{a:?} -> {b:?}");
            assert!(Navigation::clear(b, a, &collision), "{b:?} -> {a:?}");
        }
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
                    crate::game_bot::grounded_spawn(starts[1 - team][0], &collision).unwrap();
                for &spawn in spawns {
                    let start = crate::game_bot::grounded_spawn(spawn, &collision).unwrap();
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
