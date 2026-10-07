//! Server-owned motion against the exported 3.3.3.3 map geometry.
//!
//! Target evidence: Pmove 0x41e860 (66 ms chops), PM_CmdScale 0x416aa0,
//! jump edge 0x40eac0, clipMap 0x02dfb000, brush contents 0x5888e0 and
//! terrain vertex blocks 0x5881f3. The solver is a Rust swept-box implementation;
//! it does not claim exact parity with the client's capsule/edge solver.
//! Export coverage is world brushes and terrain. Static model collision and
//! moving entities are not silently replaced with bounding boxes.
use anyhow::{Context, Result, ensure};
use codol_protocol::game_usercmd::UserCommand;
use serde::Deserialize;
use std::{fs::File, io::Read, path::Path};

type V3 = [f32; 3];
pub const SPEED: u16 = 190;
pub const GRAVITY: u16 = 800;
pub const PLAYER_MASK: u32 = 0x0281_4011;
pub const WORLD: u16 = 2046;
pub const NONE: u16 = 2047;
const SKIN: f32 = 0.03125;
const CLIENT_SHA: &str = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b";

fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V3, s: f32) -> V3 {
    a.map(|x| x * s)
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}
fn normalized(a: V3) -> V3 {
    let n = length(a);
    if n > 1e-8 { mul(a, 1.0 / n) } else { [0.0; 3] }
}
fn valid(a: V3) -> bool {
    a.iter().all(|n| n.is_finite() && n.abs() <= 1_000_000.0)
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: V3,
    max: V3,
}
impl Bounds {
    fn center_half(center: V3, half: V3) -> Self {
        Self {
            min: sub(center, half),
            max: add(center, half),
        }
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }
    fn intersects(self, other: Self) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && self.max[i] >= other.min[i])
    }
    fn center(self, axis: usize) -> f32 {
        (self.min[axis] + self.max[axis]) * 0.5
    }
}

#[derive(Deserialize)]
struct Export {
    schema_version: u32,
    read_only: bool,
    client_sha256: String,
    map: String,
    ncs_crc: u32,
    brushes: Vec<BrushExport>,
    triangles: Vec<TriangleExport>,
}
#[derive(Deserialize)]
struct BrushExport {
    center: V3,
    half: V3,
    contents: u32,
    planes: Vec<[f32; 4]>,
}
#[derive(Deserialize)]
struct TriangleExport {
    vertices: [V3; 3],
    contents: u32,
    surface_flags: u32,
}

#[derive(Debug)]
enum Shape {
    Brush(Vec<[f32; 4]>),
    Triangle { vertices: [V3; 3], axes: Vec<V3> },
}
#[derive(Debug)]
struct Primitive {
    bounds: Bounds,
    contents: u32,
    surface_flags: u32,
    shape: Shape,
}
#[derive(Debug)]
enum Node {
    Leaf {
        bounds: Bounds,
        first: usize,
        count: usize,
    },
    Branch {
        bounds: Bounds,
        left: usize,
        right: usize,
    },
}
impl Node {
    fn bounds(&self) -> Bounds {
        match *self {
            Self::Leaf { bounds, .. } | Self::Branch { bounds, .. } => bounds,
        }
    }
}

#[derive(Debug)]
pub struct Collision {
    pub map: String,
    pub ncs_crc: u32,
    primitives: Vec<Primitive>,
    indices: Vec<usize>,
    nodes: Vec<Node>,
}

#[derive(Clone, Copy, Debug)]
pub struct Trace {
    pub fraction: f32,
    pub normal: V3,
    pub start_solid: bool,
    pub all_solid: bool,
    pub surface_flags: u32,
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            fraction: 1.0,
            normal: [0.0; 3],
            start_solid: false,
            all_solid: false,
            surface_flags: 0,
        }
    }
}

impl Collision {
    pub fn load(path: &Path) -> Result<Self> {
        const LIMIT: u64 = 64 * 1024 * 1024;
        let f = File::open(path)
            .with_context(|| format!("open collision export {}", path.display()))?;
        ensure!(
            f.metadata()?.len() <= LIMIT,
            "collision export exceeds 64 MiB"
        );
        let mut bytes = Vec::new();
        f.take(LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "collision export grew beyond 64 MiB"
        );
        Self::from_export(serde_json::from_slice(&bytes).context("decode collision export")?)
    }
    fn from_export(export: Export) -> Result<Self> {
        ensure!(
            export.schema_version == 1 && export.read_only && export.client_sha256 == CLIENT_SHA,
            "collision export identity mismatch"
        );
        ensure!(
            export.map.starts_with("maps/mp/")
                && export.map.ends_with(".d3dbsp")
                && export.map.len() < 256,
            "invalid collision map name"
        );
        ensure!(
            export.brushes.len() <= 65535 && export.triangles.len() <= 1_000_000,
            "collision primitive budget exceeded"
        );
        let mut primitives = Vec::new();
        for b in export.brushes {
            ensure!(
                valid(b.center)
                    && valid(b.half)
                    && b.half.iter().all(|h| *h >= 0.0)
                    && b.planes.len() <= 4096,
                "invalid collision brush"
            );
            let bounds = Bounds::center_half(b.center, b.half);
            ensure!(
                valid(bounds.min) && valid(bounds.max),
                "brush bounds out of range"
            );
            let mut planes = Vec::with_capacity(b.planes.len() + 6);
            for p in b.planes {
                let n = [p[0], p[1], p[2]];
                ensure!(
                    valid(n)
                        && p[3].is_finite()
                        && p[3].abs() <= 2_000_000.0
                        && (length(n) - 1.0).abs() < 0.005,
                    "invalid brush plane"
                );
                planes.push(p);
            }
            for i in 0..3 {
                let mut n = [0.0; 4];
                n[i] = 1.0;
                n[3] = bounds.max[i];
                planes.push(n);
                n[i] = -1.0;
                n[3] = -bounds.min[i];
                planes.push(n);
            }
            if b.contents & PLAYER_MASK != 0 {
                primitives.push(Primitive {
                    bounds,
                    contents: b.contents,
                    surface_flags: 0,
                    shape: Shape::Brush(planes),
                });
            }
        }
        for t in export.triangles {
            ensure!(
                t.vertices.iter().copied().all(valid),
                "invalid terrain vertex"
            );
            let edges = [
                sub(t.vertices[1], t.vertices[0]),
                sub(t.vertices[2], t.vertices[1]),
                sub(t.vertices[0], t.vertices[2]),
            ];
            let face = normalized(cross(edges[0], edges[1]));
            // Degenerate serialized triangles have no area and cannot collide.
            if length(face) < 0.5 || t.contents & PLAYER_MASK == 0 {
                continue;
            }
            let mut axes = vec![face, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            for e in edges {
                for axis in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
                    let n = normalized(cross(e, axis));
                    if length(n) > 0.5 {
                        axes.push(n);
                    }
                }
            }
            let bounds = Bounds {
                min: std::array::from_fn(|i| {
                    t.vertices
                        .iter()
                        .map(|v| v[i])
                        .fold(f32::INFINITY, f32::min)
                }),
                max: std::array::from_fn(|i| {
                    t.vertices
                        .iter()
                        .map(|v| v[i])
                        .fold(f32::NEG_INFINITY, f32::max)
                }),
            };
            primitives.push(Primitive {
                bounds,
                contents: t.contents,
                surface_flags: t.surface_flags,
                shape: Shape::Triangle {
                    vertices: t.vertices,
                    axes,
                },
            });
        }
        ensure!(
            !primitives.is_empty(),
            "collision export contains no player-solid geometry"
        );
        let mut collision = Self {
            map: export.map,
            ncs_crc: export.ncs_crc,
            indices: (0..primitives.len()).collect(),
            primitives,
            nodes: Vec::new(),
        };
        collision.build(0, collision.indices.len());
        Ok(collision)
    }
    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }
    fn build(&mut self, first: usize, count: usize) -> usize {
        let mut bounds = self.primitives[self.indices[first]].bounds;
        for &i in &self.indices[first + 1..first + count] {
            bounds = bounds.union(self.primitives[i].bounds);
        }
        let node = self.nodes.len();
        self.nodes.push(Node::Leaf {
            bounds,
            first,
            count,
        });
        if count > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (bounds.max[a] - bounds.min[a]).total_cmp(&(bounds.max[b] - bounds.min[b]))
                })
                .unwrap();
            self.indices[first..first + count].sort_unstable_by(|&a, &b| {
                self.primitives[a]
                    .bounds
                    .center(axis)
                    .total_cmp(&self.primitives[b].bounds.center(axis))
            });
            let left = self.build(first, count / 2);
            let right = self.build(first + count / 2, count - count / 2);
            self.nodes[node] = Node::Branch {
                bounds,
                left,
                right,
            };
        }
        node
    }
    /// Sweep an upright player hull whose origin is at its feet.
    pub fn trace(&self, start: V3, end: V3, height: f32) -> Trace {
        let half = [15.0, 15.0, height * 0.5];
        let offset = [0.0, 0.0, half[2]];
        let start = add(start, offset);
        let end = add(end, offset);
        let expanded = add(half, [SKIN; 3]);
        let bounds = Bounds::center_half(start, expanded).union(Bounds::center_half(end, expanded));
        let mut result = Trace::default();
        self.trace_node(0, bounds, start, end, half, &mut result);
        result
    }
    fn trace_node(
        &self,
        index: usize,
        bounds: Bounds,
        start: V3,
        end: V3,
        half: V3,
        result: &mut Trace,
    ) {
        if !self.nodes[index].bounds().intersects(bounds) {
            return;
        }
        match self.nodes[index] {
            Node::Branch { left, right, .. } => {
                self.trace_node(left, bounds, start, end, half, result);
                self.trace_node(right, bounds, start, end, half, result);
            }
            Node::Leaf { first, count, .. } => {
                for &i in &self.indices[first..first + count] {
                    let p = &self.primitives[i];
                    if p.contents & PLAYER_MASK == 0 || !p.bounds.intersects(bounds) {
                        continue;
                    }
                    let hit = match &p.shape {
                        Shape::Brush(planes) => sweep_brush(planes, start, end, half),
                        Shape::Triangle { vertices, axes } => {
                            sweep_triangle(*vertices, axes, start, end, half)
                        }
                    };
                    result.start_solid |= hit.start_solid;
                    result.all_solid |= hit.all_solid;
                    if hit.fraction < result.fraction {
                        result.fraction = hit.fraction;
                        result.normal = hit.normal;
                        result.surface_flags = p.surface_flags;
                    }
                }
            }
        }
    }
}

fn sweep_brush(planes: &[[f32; 4]], start: V3, end: V3, half: V3) -> Trace {
    let mut enter = -f32::INFINITY;
    let mut leave = 1.0f32;
    let mut normal = [0.0; 3];
    let mut outside = false;
    let mut end_out = false;
    for p in planes {
        let n = [p[0], p[1], p[2]];
        let radius = dot(n.map(f32::abs), half);
        let d1 = dot(n, start) - p[3] - radius;
        let d2 = dot(n, end) - p[3] - radius;
        outside |= d1 >= -0.0001;
        end_out |= d2 >= -0.0001;
        if d1 > 0.0 && d2 >= d1 {
            return Trace::default();
        }
        if d1 <= 0.0 && d2 <= 0.0 && !(d1 >= -0.0001 && d2 < d1) {
            continue;
        }
        if d1 > d2 {
            let t = ((d1 - SKIN) / (d1 - d2)).max(0.0);
            if t > enter {
                enter = t;
                normal = n;
            }
        } else {
            leave = leave.min((d1 + SKIN) / (d1 - d2));
        }
        if enter > leave {
            return Trace::default();
        }
    }
    if !outside {
        return Trace {
            fraction: if end_out { 1.0 } else { 0.0 },
            start_solid: true,
            all_solid: !end_out,
            ..Trace::default()
        };
    }
    if enter >= 0.0 && enter < leave && enter < 1.0 {
        Trace {
            fraction: enter,
            normal,
            ..Trace::default()
        }
    } else {
        Trace::default()
    }
}

/// Continuous separating-axis test. The complete axis set includes the face,
/// box axes, and triangle-edge/box-axis cross products, preventing edge leaks.
fn sweep_triangle(vertices: [V3; 3], axes: &[V3], start: V3, end: V3, half: V3) -> Trace {
    let delta = sub(end, start);
    let mut enter = -f32::INFINITY;
    let mut leave = f32::INFINITY;
    let mut normal = [0.0; 3];
    let mut penetration = f32::INFINITY;
    for &axis in axes {
        let radius = dot(axis.map(f32::abs), half);
        let min = vertices
            .iter()
            .map(|v| dot(*v, axis))
            .fold(f32::INFINITY, f32::min)
            - radius;
        let max = vertices
            .iter()
            .map(|v| dot(*v, axis))
            .fold(f32::NEG_INFINITY, f32::max)
            + radius;
        let p = dot(start, axis);
        let speed = dot(delta, axis);
        penetration = penetration.min((p - min).min(max - p));
        if speed.abs() < 1e-8 {
            if p < min || p > max {
                return Trace::default();
            }
            continue;
        }
        let (near, far, n) = if speed > 0.0 {
            ((min - p) / speed, (max - p) / speed, mul(axis, -1.0))
        } else {
            ((max - p) / speed, (min - p) / speed, axis)
        };
        if near > enter {
            enter = near;
            normal = n;
        }
        leave = leave.min(far);
        if enter > leave {
            return Trace::default();
        }
    }
    if leave < 0.0 || enter > 1.0 {
        return Trace::default();
    }
    if enter < -0.00001 {
        // Touching a face while moving tangentially is not penetration.
        return if penetration > 0.001 {
            Trace {
                fraction: 0.0,
                start_solid: true,
                all_solid: leave >= 1.0,
                ..Trace::default()
            }
        } else {
            Trace::default()
        };
    }
    let approach = -dot(delta, normal);
    if approach <= 1e-8 {
        return Trace::default();
    }
    Trace {
        fraction: (enter - SKIN / approach).max(0.0),
        normal,
        ..Trace::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stance {
    #[default]
    Stand,
    Crouch,
    Prone,
}
impl Stance {
    pub fn height(self) -> f32 {
        match self {
            Self::Stand => 70.0,
            Self::Crouch => 50.0,
            Self::Prone => 30.0,
        }
    }
    pub fn view_height(self) -> u8 {
        match self {
            Self::Stand => 60,
            Self::Crouch => 40,
            Self::Prone => 11,
        }
    }
    fn flags(self) -> u32 {
        match self {
            Self::Stand => 0,
            Self::Crouch => 2,
            Self::Prone => 1,
        }
    }
    fn scale(self) -> f32 {
        match self {
            Self::Stand => 1.0,
            Self::Crouch => 0.65,
            Self::Prone => 0.15,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Motion {
    pub origin: V3,
    pub velocity: V3,
    pub command_time: u32,
    pub speed: u16,
    pub gravity: u16,
    pub stance: Stance,
    pub flags: u32,
    pub ground_entity: u16,
    pub jump_time: u32,
    pub jump_origin_z: f32,
    pub view_height: f32,
    pub view_height_target: u8,
    pub view_height_lerp_time: u32,
    pub view_height_lerp_target: u8,
    pub view_height_lerp_down: bool,
    pub bob_cycle: u8,
    pub surface_flags: u32,
    previous_buttons: u32,
    ground_normal: V3,
    view_start: f32,
}
impl Motion {
    pub fn new(origin: V3) -> Self {
        Self {
            origin,
            velocity: [0.0; 3],
            command_time: 0,
            speed: SPEED,
            gravity: GRAVITY,
            stance: Stance::Stand,
            flags: 0,
            ground_entity: NONE,
            jump_time: 0,
            jump_origin_z: 0.0,
            view_height: 60.0,
            view_height_target: 60,
            view_height_lerp_time: 0,
            view_height_lerp_target: 60,
            view_height_lerp_down: false,
            bob_cycle: 0,
            surface_flags: 0,
            previous_buttons: 0,
            ground_normal: [0.0, 0.0, 1.0],
            view_start: 60.0,
        }
    }
    pub fn grounded(&self) -> bool {
        self.ground_entity != NONE
    }
    /// `weapon_scale` is the currently selected weapon's hip/ADS scale.
    /// Command time belongs to input, never the snapshot clock.
    pub fn apply(
        &mut self,
        cmd: &UserCommand,
        weapon_scale: f32,
        ads: bool,
        collision: &Collision,
    ) {
        if cmd.time <= self.command_time || !weapon_scale.is_finite() || weapon_scale < 0.0 {
            return;
        }
        if cmd.time - self.command_time > 1000 {
            self.command_time = cmd.time - 1000;
        }
        while self.command_time < cmd.time {
            let dt_ms = (cmd.time - self.command_time).min(66);
            self.command_time += dt_ms;
            self.step(cmd, weapon_scale, ads, dt_ms, collision);
            self.previous_buttons = cmd.buttons;
        }
    }
    fn step(&mut self, cmd: &UserCommand, weapon_scale: f32, ads: bool, dt_ms: u32, c: &Collision) {
        let dt = dt_ms as f32 * 0.001;
        let desired = if cmd.buttons & 0x100 != 0 {
            Stance::Prone
        } else if cmd.buttons & 0x200 != 0 {
            Stance::Crouch
        } else {
            Stance::Stand
        };
        if desired != self.stance
            && (desired.height() < self.stance.height()
                || !c
                    .trace(self.origin, self.origin, desired.height())
                    .start_solid)
        {
            self.stance = desired;
            self.view_start = self.view_height;
            self.view_height_target = desired.view_height();
            self.view_height_lerp_target = desired.view_height();
            self.view_height_lerp_time = self.command_time;
            self.view_height_lerp_down = self.view_height > f32::from(self.view_height_target);
        }
        self.update_view_height();
        self.ground(c, 0.25);
        let jumping = self.grounded()
            && self.stance == Stance::Stand
            && cmd.buttons & 0x400 != 0
            && self.previous_buttons & 0x400 == 0
            && self.command_time.saturating_sub(self.jump_time) >= 500;
        if jumping {
            self.velocity[2] = (2.0 * f32::from(self.gravity) * 39.0).sqrt();
            self.jump_time = self.command_time;
            self.jump_origin_z = self.origin[2];
            self.ground_entity = NONE;
        }
        let locked = cmd.buttons & (0x1000_0000 | 0x0020_0000) != 0;
        let f = if locked {
            0.0
        } else if cmd.direction & 1 != 0 {
            127.0
        } else if cmd.direction & 2 != 0 {
            -127.0
        } else {
            0.0
        };
        let mut r = if locked {
            0.0
        } else if cmd.direction & 4 != 0 {
            127.0
        } else if cmd.direction & 8 != 0 {
            -127.0
        } else {
            0.0
        };
        let sprint = cmd.buttons & 2 != 0 && f > 0.0 && !ads && self.stance == Stance::Stand;
        self.flags = self.stance.flags()
            | if sprint { 0x4000 } else { 0 }
            | if !self.grounded() && self.jump_time != 0 {
                0x2000
            } else {
                0
            };
        if sprint && self.grounded() {
            // 0x4174ed..0x417502, double constant 0xc862d0.
            r = (r * 0.667f32).trunc();
        }
        let horizontal =
            (self.velocity[0] * self.velocity[0] + self.velocity[1] * self.velocity[1]).sqrt();
        if self.grounded() {
            if horizontal < 1.0 {
                self.velocity[0] = 0.0;
                self.velocity[1] = 0.0;
            } else {
                let scale = ((horizontal - horizontal.max(100.0) * 5.5 * dt).max(0.0)) / horizontal;
                self.velocity[0] *= scale;
                self.velocity[1] *= scale;
            }
        }
        let yaw = f32::from(cmd.angles[1]) * (std::f32::consts::TAU / 65536.0);
        let wish = [
            yaw.cos() * f + yaw.sin() * r,
            yaw.sin() * f - yaw.cos() * r,
            0.0,
        ];
        let total = (f * f + r * r).sqrt();
        let biggest = (if f < 0.0 { f * 0.7 } else { f })
            .abs()
            .max((r * 0.8).abs());
        let speed = if total > 0.0 {
            f32::from(self.speed) * biggest / 127.0
                * weapon_scale
                * self.stance.scale()
                * if ads { 0.4 } else { 1.0 }
                * if sprint { 1.5 } else { 1.0 }
        } else {
            0.0
        };
        let mut direction = normalized(wish);
        if self.grounded() {
            direction = project_ground(direction, self.ground_normal);
        }
        let acceleration = if !self.grounded() {
            1.0
        } else {
            match self.stance {
                Stance::Stand => 9.0,
                Stance::Crouch => 12.0,
                Stance::Prone => 19.0,
            }
        };
        let amount = (speed - dot(self.velocity, direction))
            .max(0.0)
            .min(dt * acceleration * speed.max(100.0));
        self.velocity = add(self.velocity, mul(direction, amount));
        if self.grounded() {
            self.velocity = project_ground(self.velocity, self.ground_normal);
        }
        let was_grounded = self.grounded();
        if was_grounded && horizontal > 1.0 {
            self.bob_cycle = self
                .bob_cycle
                .wrapping_add((horizontal / f32::from(self.speed) * dt_ms as f32 * 0.4) as u8);
        }
        self.step_slide(c, dt, !was_grounded);
        self.ground(c, if was_grounded { 9.0 } else { 0.25 });
    }
    fn update_view_height(&mut self) {
        if self.view_height_lerp_time == 0 {
            return;
        }
        let duration = if self.view_start < 40.0 || self.view_height_target == 11 {
            400.0
        } else {
            200.0
        };
        let t = (self.command_time - self.view_height_lerp_time) as f32 / duration;
        if t >= 1.0 {
            self.view_height = f32::from(self.view_height_target);
            self.view_height_lerp_time = 0;
        } else {
            let smooth = t * t * (3.0 - 2.0 * t);
            self.view_height =
                self.view_start + (f32::from(self.view_height_target) - self.view_start) * smooth;
        }
    }
    fn ground(&mut self, c: &Collision, distance: f32) {
        let trace = c.trace(
            self.origin,
            add(self.origin, [0.0, 0.0, -distance]),
            self.stance.height(),
        );
        // A slope change can increase world-space vertical velocity while the
        // player remains tangent to the new surface. Test the freshly traced
        // plane, not the preceding frame's normal (which falsely launches the
        // player and lets gravity accumulate sideways drift on Afghan terrain).
        if trace.fraction < 1.0 && self.velocity[2] > 0.0 && dot(self.velocity, trace.normal) > 10.0
        {
            self.ground_entity = NONE;
            return;
        }
        if trace.fraction < 1.0 && !trace.start_solid && trace.normal[2] >= 0.7 {
            self.origin[2] -= distance * trace.fraction;
            self.ground_entity = WORLD;
            self.ground_normal = trace.normal;
            self.surface_flags = trace.surface_flags;
            self.velocity = project_ground(self.velocity, trace.normal);
            self.flags &= !0x2000;
        } else {
            self.ground_entity = NONE;
        }
    }
    fn step_slide(&mut self, c: &Collision, dt: f32, gravity: bool) {
        let start = self.origin;
        let velocity = self.velocity;
        let blocked = self.slide(c, dt, gravity);
        if !blocked || gravity {
            return;
        }
        let down_origin = self.origin;
        let down_velocity = self.velocity;
        let height = if self.stance == Stance::Prone {
            10.0
        } else {
            18.0
        };
        let up = c.trace(start, add(start, [0.0, 0.0, height]), self.stance.height());
        if up.start_solid || up.fraction == 0.0 {
            return;
        }
        self.origin = add(start, [0.0, 0.0, height * up.fraction]);
        self.velocity = velocity;
        self.slide(c, dt, gravity);
        let down = c.trace(
            self.origin,
            add(self.origin, [0.0, 0.0, -height * up.fraction - 9.0]),
            self.stance.height(),
        );
        let step_distance =
            (self.origin[0] - start[0]).powi(2) + (self.origin[1] - start[1]).powi(2);
        let flat_distance =
            (down_origin[0] - start[0]).powi(2) + (down_origin[1] - start[1]).powi(2);
        if down.start_solid
            || down.fraction == 1.0
            || down.normal[2] < 0.7
            || step_distance <= flat_distance
        {
            self.origin = down_origin;
            self.velocity = down_velocity;
        } else {
            self.origin[2] -= (height * up.fraction + 9.0) * down.fraction;
            self.velocity = project_ground(self.velocity, down.normal);
        }
    }
    fn slide(&mut self, c: &Collision, dt: f32, gravity: bool) -> bool {
        let mut end_velocity = self.velocity;
        if gravity {
            end_velocity[2] -= f32::from(self.gravity) * dt;
            self.velocity[2] = (self.velocity[2] + end_velocity[2]) * 0.5;
        }
        let mut planes = Vec::with_capacity(8);
        if self.grounded() {
            planes.push(self.ground_normal);
        }
        let mut time = dt;
        let mut blocked = false;
        for _ in 0..4 {
            let trace = c.trace(
                self.origin,
                add(self.origin, mul(self.velocity, time)),
                self.stance.height(),
            );
            if trace.all_solid {
                self.velocity = [0.0; 3];
                return true;
            }
            self.origin = add(self.origin, mul(self.velocity, time * trace.fraction));
            if trace.fraction == 1.0 {
                break;
            }
            blocked = true;
            time *= 1.0 - trace.fraction;
            if planes.iter().any(|&n| dot(n, trace.normal) > 0.999) {
                self.velocity = add(clip(self.velocity, trace.normal), mul(trace.normal, 0.01));
                continue;
            }
            planes.push(trace.normal);
            self.velocity = clip_planes(self.velocity, &planes);
            end_velocity = clip_planes(end_velocity, &planes);
            if length(self.velocity) < 0.001 {
                break;
            }
        }
        if gravity {
            self.velocity = end_velocity;
        }
        blocked
    }
}
fn clip(v: V3, n: V3) -> V3 {
    let d = dot(v, n);
    sub(v, mul(n, if d < 0.0 { d * 1.001 } else { d / 1.001 }))
}
fn project_ground(v: V3, n: V3) -> V3 {
    if n[2] < 0.001 {
        return clip(v, n);
    }
    let z = -(v[0] * n[0] + v[1] * n[1]) / n[2];
    let result = [v[0], v[1], z];
    let before = length(v);
    let after = length(result);
    if after > 0.0 {
        mul(result, before / after)
    } else {
        [0.0; 3]
    }
}
fn clip_planes(v: V3, planes: &[V3]) -> V3 {
    let mut result = v;
    for (i, &n) in planes.iter().enumerate() {
        if dot(result, n) >= 0.0 {
            continue;
        }
        result = clip(result, n);
        for (j, &other) in planes.iter().enumerate() {
            if i == j || dot(result, other) >= 0.0 {
                continue;
            }
            result = clip(result, other);
            if dot(result, n) < 0.0 {
                let crease = normalized(cross(n, other));
                result = mul(crease, dot(v, crease));
            }
            if planes.iter().any(|&p| dot(result, p) < -0.01) {
                return [0.0; 3];
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world(brushes: Vec<BrushExport>, triangles: Vec<TriangleExport>) -> Collision {
        Collision::from_export(Export {
            schema_version: 1,
            read_only: true,
            client_sha256: CLIENT_SHA.into(),
            map: "maps/mp/test.d3dbsp".into(),
            ncs_crc: 0,
            brushes,
            triangles,
        })
        .unwrap()
    }
    fn brush(center: V3, half: V3) -> BrushExport {
        BrushExport {
            center,
            half,
            contents: 1,
            planes: vec![],
        }
    }
    fn floor() -> BrushExport {
        brush([0.0, 0.0, -20.0], [2000.0, 2000.0, 20.0])
    }
    fn cmd(time: u32, direction: u8, buttons: u32) -> UserCommand {
        UserCommand {
            time,
            direction,
            buttons,
            ..Default::default()
        }
    }
    fn tick(m: &mut Motion, c: &Collision, direction: u8, buttons: u32, n: usize) {
        for _ in 0..n {
            m.apply(&cmd(m.command_time + 16, direction, buttons), 1.0, false, c);
        }
    }
    #[test]
    fn wall_stops_forward_motion_but_allows_sliding() {
        let c = world(
            vec![floor(), brush([100.0, 0.0, 100.0], [10.0, 1000.0, 100.0])],
            vec![],
        );
        let mut m = Motion::new([0.0, 0.0, 2.0]);
        tick(&mut m, &c, 5, 0, 150);
        assert!(m.origin[0] <= 75.01, "{:?}", m.origin);
        assert!(m.origin[1] < -100.0);
        assert!(m.grounded());
    }
    #[test]
    fn jump_returns_to_real_ground_and_does_not_repeat_while_held() {
        let c = world(vec![floor()], vec![]);
        let mut m = Motion::new([0.0, 0.0, 2.0]);
        tick(&mut m, &c, 0, 0, 40);
        let base = m.origin[2];
        tick(&mut m, &c, 0, 0x400, 12);
        assert!(m.origin[2] > base + 20.0, "{:?}", m.origin);
        tick(&mut m, &c, 0, 0x400, 100);
        assert!(m.grounded());
        assert!((m.origin[2] - base).abs() < 0.1);
    }
    #[test]
    fn crouch_cannot_stand_through_a_ceiling() {
        let c = world(
            vec![floor(), brush([0.0, 0.0, 65.0], [80.0, 80.0, 5.0])],
            vec![],
        );
        let mut m = Motion::new([0.0, 0.0, 1.0]);
        tick(&mut m, &c, 0, 0x200, 40);
        assert_eq!(m.stance, Stance::Crouch);
        tick(&mut m, &c, 0, 0, 40);
        assert_eq!(m.stance, Stance::Crouch);
        assert_eq!(m.view_height, 40.0);
        tick(&mut m, &c, 1, 0x200, 100);
        tick(&mut m, &c, 0, 0, 40);
        assert_eq!(m.stance, Stance::Stand);
    }
    #[test]
    fn can_step_over_a_twelve_unit_obstacle() {
        let c = world(
            vec![floor(), brush([80.0, 0.0, 6.0], [15.0, 80.0, 6.0])],
            vec![],
        );
        let mut m = Motion::new([0.0, 0.0, 1.0]);
        tick(&mut m, &c, 1, 0, 100);
        assert!(m.origin[0] > 150.0, "{:?}", m.origin);
    }
    #[test]
    fn triangle_slope_supports_a_falling_hull() {
        let c = world(
            vec![],
            vec![TriangleExport {
                vertices: [
                    [-500.0, -500.0, -125.0],
                    [500.0, -500.0, 125.0],
                    [0.0, 500.0, 0.0],
                ],
                contents: 1,
                surface_flags: 7,
            }],
        );
        let mut m = Motion::new([0.0, 0.0, 100.0]);
        tick(&mut m, &c, 0, 0, 100);
        assert!(m.grounded(), "{:?}", m);
        assert!(
            (m.origin[2] - m.origin[0] * 0.25 - 3.75).abs() < 0.1,
            "{:?}",
            m.origin
        );
        assert_eq!(m.surface_flags, 7);
        tick(&mut m, &c, 1, 0, 40);
        assert!(m.origin[0] > 20.0);
        assert!(m.origin[2] > 8.0);
    }
    #[test]
    fn changing_slope_uses_the_new_ground_normal() {
        let c = world(
            vec![],
            vec![TriangleExport {
                vertices: [
                    [-500.0, -500.0, -125.0],
                    [500.0, -500.0, 125.0],
                    [0.0, 500.0, 0.0],
                ],
                contents: 1,
                surface_flags: 0,
            }],
        );
        let mut m = Motion::new([0.0, 0.0, 3.8]);
        m.velocity = [184.0, 0.0, 46.0];
        m.ground_entity = WORLD;
        m.ground(&c, 0.25);
        assert!(m.grounded());
        assert!(m.velocity[1].abs() < 0.001);
    }
    #[test]
    fn commands_are_deduplicated_and_air_does_not_invent_a_floor() {
        let c = world(vec![brush([1000.0, 1000.0, -20.0], [1.0; 3])], vec![]);
        let mut m = Motion::new([0.0, 0.0, 20.0]);
        m.apply(&cmd(66, 1, 0), 1.0, false, &c);
        let once = m.origin;
        m.apply(&cmd(66, 1, 0), 1.0, false, &c);
        assert_eq!(m.origin, once);
        tick(&mut m, &c, 0, 0, 100);
        assert!(m.origin[2] < -100.0);
        assert!(!m.grounded());
    }
    #[test]
    fn continuous_sweep_blocks_fast_motion_and_respects_slanted_brush_plane() {
        let mut b = brush([100.0, 0.0, 50.0], [100.0, 100.0, 50.0]);
        let s = 2f32.sqrt().recip();
        b.planes.push([-s, 0.0, s, 0.0]);
        let c = world(vec![b], vec![]);
        let trace = c.trace([-100.0, 0.0, 20.0], [1000.0, 0.0, 20.0], 70.0);
        assert!(trace.fraction < 0.2);
        assert!(
            trace.normal[0] < -0.5 && trace.normal[2] > 0.5,
            "{:?}",
            trace
        );
    }
}
