//! Continuous bounds sweeps for fixed-orientation reward objects.
use super::*;

impl Collision {
    /// Sweep the complete local AABB against projectile-visible geometry and
    /// all dynamic groups. Player-only clips are navigation barriers, not
    /// surfaces on which airborne reward objects can land.
    pub(crate) fn trace_bounds(
        &self,
        start: V3,
        end: V3,
        min: V3,
        max: V3,
        ignore: Option<u16>,
    ) -> Trace {
        self.trace_bounds_mask(start, end, min, max, ignore, SHOT_MASK)
    }
    /// Ground animals use the shared walking mask with their own full bounds.
    pub(crate) fn trace_actor_bounds(
        &self,
        start: V3,
        end: V3,
        min: V3,
        max: V3,
        ignore: Option<u16>,
    ) -> Trace {
        self.trace_bounds_mask(start, end, min, max, ignore, PLAYER_MASK)
    }
    fn trace_bounds_mask(
        &self,
        start: V3,
        end: V3,
        min: V3,
        max: V3,
        ignore: Option<u16>,
        mask: u32,
    ) -> Trace {
        if ![start, end, min, max].into_iter().all(valid) || !(0..3).all(|i| min[i] < max[i]) {
            return Trace {
                fraction: 0.0,
                start_solid: true,
                all_solid: true,
                ..Trace::default()
            };
        }
        let offset = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let half = std::array::from_fn(|i| (max[i] - min[i]) * 0.5);
        let first = add(start, offset);
        let last = add(end, offset);
        let swept = Bounds {
            min: std::array::from_fn(|i| first[i].min(last[i]) - half[i]),
            max: std::array::from_fn(|i| first[i].max(last[i]) + half[i]),
        };
        let mut result = Trace::default();
        let mut nodes = vec![0];
        while let Some(index) = nodes.pop() {
            if !self.nodes[index].bounds().intersects(swept) {
                continue;
            }
            match self.nodes[index] {
                Node::Branch { left, right, .. } => {
                    nodes.push(left);
                    nodes.push(right);
                }
                Node::Leaf {
                    first: index,
                    count,
                    ..
                } => {
                    for &id in &self.indices[index..index + count] {
                        let p = &self.primitives[id];
                        if p.contents & mask == 0 || self.glass_is_absent(p.glass_piece) {
                            continue;
                        }
                        merge(&mut result, sweep(p, first, last, half), p, None);
                    }
                }
            }
        }
        for (entity, p) in self.props.read().expect("prop geometry lock").iter() {
            if Some(*entity) != ignore && p.bounds.intersects(swept) {
                merge(&mut result, sweep(p, first, last, half), p, Some(*entity));
            }
        }
        result
    }
}
fn merge(result: &mut Trace, hit: Trace, p: &Primitive, entity: Option<u16>) {
    let start_solid = result.start_solid || hit.start_solid;
    let all_solid = result.all_solid || hit.all_solid;
    if hit.fraction < result.fraction || (hit.start_solid && !result.start_solid) {
        *result = hit;
        result.contents = p.contents;
        result.prop_entity = entity;
    }
    result.start_solid = start_solid;
    result.all_solid = all_solid;
}
fn sweep(p: &Primitive, start: V3, end: V3, half: V3) -> Trace {
    match &p.shape {
        Shape::Brush { planes, surfaces } => {
            let expanded = planes
                .iter()
                .map(|plane| {
                    let support = (0..3).map(|i| plane[i].abs() * half[i]).sum::<f32>();
                    [plane[0], plane[1], plane[2], plane[3] + support]
                })
                .collect::<Vec<_>>();
            ray_brush(&expanded, surfaces, start, end).unwrap_or_default()
        }
        Shape::Triangle { vertices } => {
            let mut t = sweep_triangle_box(*vertices, start, end, half);
            t.surface_flags = p.surface_flags;
            t
        }
    }
}
/// Swept separating axes for the triangle/AABB Minkowski volume. Testing
/// face normal, box axes and nine edge cross axes also covers terrain edges.
fn sweep_triangle_box(vertices: [V3; 3], start: V3, end: V3, half: V3) -> Trace {
    let edges = [
        sub(vertices[1], vertices[0]),
        sub(vertices[2], vertices[1]),
        sub(vertices[0], vertices[2]),
    ];
    let basis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut axes = Vec::with_capacity(13);
    axes.extend(basis);
    axes.push(cross(edges[0], edges[1]));
    for edge in edges {
        for axis in basis {
            axes.push(cross(edge, axis));
        }
    }
    let mut enter = 0.0f64;
    let mut leave = 1.0f64;
    let mut normal = [0.0; 3];
    let mut inside = true;
    let mut end_inside = true;
    for axis in axes {
        if length(axis) < 0.00001 {
            continue;
        }
        let axis = normalized(axis);
        let radius = (0..3)
            .map(|i| f64::from(axis[i].abs()) * f64::from(half[i]))
            .sum::<f64>();
        let projection = |point: V3| {
            (0..3)
                .map(|i| f64::from(point[i]) * f64::from(axis[i]))
                .sum::<f64>()
        };
        let low = vertices
            .map(projection)
            .into_iter()
            .fold(f64::INFINITY, f64::min)
            - radius;
        let high = vertices
            .map(projection)
            .into_iter()
            .fold(f64::NEG_INFINITY, f64::max)
            + radius;
        let a = projection(start);
        let b = projection(end);
        let speed = b - a;
        inside &= a > low + 0.00001 && a < high - 0.00001;
        end_inside &= b > low + 0.00001 && b < high - 0.00001;
        if speed.abs() < 1e-10 {
            if a < low || a > high {
                return Trace::default();
            }
            continue;
        }
        let first = (low - a) / speed;
        let second = (high - a) / speed;
        let (near, far, n) = if first <= second {
            (first, second, axis.map(|v| -v))
        } else {
            (second, first, axis)
        };
        if near >= enter {
            enter = near;
            normal = n;
        }
        leave = leave.min(far);
        if enter > leave {
            return Trace::default();
        }
    }
    if inside {
        return Trace {
            fraction: 0.0,
            start_solid: true,
            all_solid: end_inside,
            ..Trace::default()
        };
    }
    if enter < 1.0 && leave > 0.0 && length(normal) > 0.5 {
        Trace {
            fraction: enter.max(0.0) as f32,
            normal,
            ..Trace::default()
        }
    } else {
        Trace::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn falling_box_hits_a_terrain_edge_that_a_center_ray_misses() {
        let vertices = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
        let trace = sweep_triangle_box(
            vertices,
            [55.0, 55.0, 100.0],
            [55.0, 55.0, -100.0],
            [10.0, 10.0, 10.0],
        );
        assert!((trace.fraction - 0.45).abs() < 0.001);
        assert!(trace.normal[2] > 0.9);
        assert_eq!(
            sweep_triangle_box(
                vertices,
                [120.0, 120.0, 100.0],
                [120.0, 120.0, -100.0],
                [10.0; 3]
            )
            .fraction,
            1.0
        );
    }
    #[test]
    fn falling_reward_ignores_player_clip_but_stops_on_solid_floor() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "read_only": true, "client_sha256": CLIENT_SHA,
            "map": "maps/mp/test.d3dbsp", "ncs_crc": 0,
            "brushes": [
                {"center": [0,0,1100], "half": [100,100,100], "contents": 0x8030200u32, "planes": []},
                {"center": [0,0,-10], "half": [100,100,10], "contents": 1, "planes": []}
            ], "triangles": []
        })).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        let trace = collision.trace_bounds(
            [0.0, 0.0, 1300.0],
            [0.0, 0.0, -100.0],
            [-10.0; 3],
            [10.0; 3],
            None,
        );
        assert!((trace.fraction - 1290.0 / 1400.0).abs() < 0.001);
        assert_eq!(trace.contents, 1);
        assert!(trace.normal[2] > 0.9);
        let actor = collision.trace_actor_bounds(
            [0.0, 0.0, 1300.0],
            [0.0, 0.0, -100.0],
            [-10.0; 3],
            [10.0; 3],
            None,
        );
        assert!(actor.fraction < 0.1);
        assert_ne!(actor.contents & PLAYER_MASK, 0);
    }
}
