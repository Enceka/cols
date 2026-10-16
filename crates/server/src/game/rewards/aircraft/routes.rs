use super::*;
use crate::game::rewards::paths::{FlightNode, FlightNodes};

// Explicit compatibility conversion from Alpha's mph-based vehicle loader.
// Movement stores inches/second; independent target script-consumer evidence
// is still required before calling these recovered3.3.3.3 movement units.
const MPH: f32 = 17.6;
#[derive(Clone)]
pub(in crate::game::rewards) struct Waypoint {
    pub origin: [f32; 3],
    pub speed: f32,
    pub accel: f32,
    pub delay: u32,
    pub yaw: Option<f32>,
}
#[derive(Clone)]
pub(in crate::game::rewards) struct Path {
    pub nodes: Vec<Waypoint>,
    pub repeat: Option<usize>,
}
impl Waypoint {
    fn from_node(n: &FlightNode, heavy: bool) -> Self {
        let scale = if heavy { 0.5 } else { 1.0 };
        Self {
            origin: [n.origin[0], n.origin[1], n.origin[2] + 30.0],
            speed: n.script_airspeed.unwrap_or(40.0) * MPH * scale,
            accel: n.script_accel.unwrap_or(22.0) * MPH * scale,
            delay: (n.script_delay.unwrap_or(0.0) * 1000.0).round() as u32,
            yaw: n.angles.map(|a| a[1]),
        }
    }
}
pub(in crate::game::rewards) fn envelope(bounds: ModelBounds) -> ([f32; 3], [f32; 3]) {
    let x = bounds.min[0].abs().max(bounds.max[0].abs());
    let y = bounds.min[1].abs().max(bounds.max[1].abs());
    let radius = x.hypot(y);
    (
        [-radius, -radius, bounds.min[2]],
        [radius, radius, bounds.max[2]],
    )
}
fn clear(
    c: &Collision,
    bounds: ModelBounds,
    a: [f32; 3],
    b: [f32; 3],
    ignore: Option<u16>,
) -> bool {
    let (min, max) = envelope(bounds);
    let t = c.trace_bounds(a, b, min, max, ignore);
    !t.start_solid && !t.all_solid && t.fraction >= 1.0
}
/// Reference getOriginOffsets raises obstructed endpoints in128-unit steps.
/// Every accepted leg is checked with the complete yaw envelope, including
/// turn-in-place geometry, rather than a center ray or isolated samples.
pub(in crate::game::rewards) fn fit(
    c: &Collision,
    bounds: ModelBounds,
    path: &mut Path,
    start: Option<[f32; 3]>,
    ignore: Option<u16>,
) -> bool {
    if path.nodes.is_empty() {
        return false;
    }
    for lift in 0..=16 {
        let z = lift as f32 * 128.0;
        let points: Vec<_> = path
            .nodes
            .iter()
            .map(|n| [n.origin[0], n.origin[1], n.origin[2] + z])
            .collect();
        if points.iter().any(|p| p[2].abs() > 30000.0) {
            continue;
        }
        let first = start.unwrap_or(points[0]);
        if !clear(c, bounds, first, points[0], ignore)
            || !points
                .windows(2)
                .all(|w| clear(c, bounds, w[0], w[1], ignore))
        {
            continue;
        }
        if path.repeat.is_some_and(|repeat| {
            !clear(c, bounds, *points.last().unwrap(), points[repeat], ignore)
        }) {
            continue;
        }
        for (node, p) in path.nodes.iter_mut().zip(points) {
            node.origin = p;
        }
        return true;
    }
    false
}
pub(in crate::game::rewards) fn authored(
    g: &FlightNodes,
    index: usize,
    heavy: bool,
) -> Option<Path> {
    let route = g.route(index)?;
    Some(Path {
        nodes: route
            .nodes
            .into_iter()
            .map(|i| Waypoint::from_node(&g.nodes[i], heavy))
            .collect(),
        repeat: route.repeat_from,
    })
}
pub(in crate::game::rewards) fn exit_path(
    origin: [f32; 3],
    g: &FlightNodes,
    bounds: ModelBounds,
    c: &Collision,
    entity: Option<u16>,
) -> Option<Path> {
    let mut leaves = g.entries.get("heli_leave")?.clone();
    leaves.sort_by(|&a, &b| {
        distance(origin, g.nodes[a].origin).total_cmp(&distance(origin, g.nodes[b].origin))
    });
    for node in leaves {
        let goal = Waypoint::from_node(&g.nodes[node], false);
        let mut p = Path {
            nodes: vec![goal.clone()],
            repeat: None,
        };
        if fit(c, bounds, &mut p, Some(origin), entity) {
            return Some(p);
        }
        for rise in (128..=2048).step_by(128) {
            let mut p = Path {
                nodes: vec![
                    Waypoint {
                        origin: [origin[0], origin[1], origin[2] + rise as f32],
                        ..goal.clone()
                    },
                    goal.clone(),
                ],
                repeat: None,
            };
            if fit(c, bounds, &mut p, Some(origin), entity) {
                return Some(p);
            }
        }
    }
    None
}
/// Crash routes keep their authored descent: lifting a terminal crash point
/// would turn destruction into another patrol. The final leg may hit terrain;
/// the live full-body sweep supplies the actual impact position.
pub(in crate::game::rewards) fn crash_path(
    origin: [f32; 3],
    g: &FlightNodes,
    bounds: ModelBounds,
    c: &Collision,
    entity: u16,
) -> Option<Path> {
    let mut roots = g.entries.get("heli_crash_start")?.clone();
    roots.sort_by(|&a, &b| {
        distance(origin, g.nodes[a].origin).total_cmp(&distance(origin, g.nodes[b].origin))
    });
    for root in roots {
        let path = authored(g, root, false)?;
        if path.repeat.is_some() || path.nodes.len() < 2 {
            continue;
        }
        if clear(c, bounds, origin, path.nodes[0].origin, Some(entity))
            && path.nodes[..path.nodes.len() - 1]
                .windows(2)
                .all(|w| clear(c, bounds, w[0].origin, w[1].origin, Some(entity)))
        {
            return Some(path);
        }
    }
    None
}
pub(in crate::game::rewards) struct Motion {
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub path: Path,
    pub next: usize,
    pub speed: f32,
    pub wait_until: u32,
}
impl Motion {
    pub fn new(path: Path) -> Self {
        let origin = path.nodes[0].origin;
        let yaw = path.nodes[0].yaw.unwrap_or_else(|| {
            path.nodes.get(1).map_or(0.0, |n| {
                (n.origin[1] - origin[1])
                    .atan2(n.origin[0] - origin[0])
                    .to_degrees()
            })
        });
        Self {
            origin,
            angles: [0, short(yaw), 0],
            path,
            next: 1,
            speed: 0.0,
            wait_until: 0,
        }
    }
    pub fn replace(&mut self, path: Path) {
        self.path = path;
        self.next = 0;
        self.wait_until = 0;
    }
    pub fn step(
        &mut self,
        time: u32,
        dt: f32,
        bounds: ModelBounds,
        entity: u16,
        c: &Collision,
    ) -> Movement {
        if time < self.wait_until {
            self.speed = 0.0;
            return Movement::Moving;
        }
        if self.next >= self.path.nodes.len() {
            if let Some(repeat) = self.path.repeat {
                self.next = repeat;
            } else {
                return Movement::Done;
            }
        }
        let node = &self.path.nodes[self.next];
        let delta: [f32; 3] = std::array::from_fn(|i| node.origin[i] - self.origin[i]);
        let d = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
        let braking = (2.0 * node.accel.max(1.0) * d).sqrt();
        let desired = node.speed.min(braking);
        let change = node.accel * dt;
        self.speed += (desired - self.speed).clamp(-change, change);
        let travel = (self.speed * dt).min(d);
        let next = if d < 0.01 {
            node.origin
        } else {
            std::array::from_fn(|i| self.origin[i] + delta[i] * travel / d)
        };
        let (min, max) = envelope(bounds);
        let trace = c.trace_bounds(self.origin, next, min, max, Some(entity));
        if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
            if !trace.start_solid && !trace.all_solid {
                self.origin = std::array::from_fn(|i| {
                    self.origin[i] + (next[i] - self.origin[i]) * trace.fraction
                });
            }
            return Movement::Blocked;
        }
        self.origin = next;
        if delta[0].hypot(delta[1]) > 0.1 {
            let yaw = node.yaw.unwrap_or(delta[1].atan2(delta[0]).to_degrees());
            let current = f32::from(self.angles[1]) * 360.0 / 65536.0;
            let turn = (yaw - current + 180.0).rem_euclid(360.0) - 180.0;
            self.angles[1] = short(current + turn.clamp(-75.0 * dt, 75.0 * dt));
        }
        if d <= travel + 1.0 {
            self.origin = node.origin;
            self.wait_until = time.saturating_add(node.delay);
            self.next += 1;
        }
        Movement::Moving
    }
}
pub(in crate::game::rewards) enum Movement {
    Moving,
    Done,
    Blocked,
}
