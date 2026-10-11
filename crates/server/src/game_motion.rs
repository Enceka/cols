//! Server-owned motion against the exported 3.3.3.3 map geometry.
//!
//! Target evidence: Pmove 0x41e860 (66 ms chops), PM_CmdScale 0x416aa0,
//! jump edge 0x40eac0, clipMap 0x02dfb000, brush contents 0x5888e0 and
//! terrain vertex blocks 0x5881f3. The upright capsule has radius 15 and a
//! vertical center segment of length `height - 30`. Brush planes use that
//! capsule's support function; terrain sweeps its full face/edge/vertex distance.
//! This does not claim exact parity with the client's mesh-border filtering.
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
/// Target 3.3.3.3 bullet trace mask (pushes at 0x55a4a0 / 0x55a4c3).
/// It includes clipshot geometry and excludes player-only clip volumes.
pub const SHOT_MASK: u32 = 0x0280_e831;
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
    #[serde(default = "legacy_contents_mask")]
    captured_contents_mask: u32,
    brushes: Vec<BrushExport>,
    triangles: Vec<TriangleExport>,
}
fn legacy_contents_mask() -> u32 {
    PLAYER_MASK
}
#[derive(Deserialize)]
struct BrushExport {
    center: V3,
    half: V3,
    contents: u32,
    planes: Vec<[f32; 4]>,
    /// Side planes followed by +X,-X,+Y,-Y,+Z,-Z bound faces.
    #[serde(default)]
    plane_surface_flags: Vec<u32>,
}
#[derive(Deserialize)]
struct TriangleExport {
    vertices: [V3; 3],
    contents: u32,
    surface_flags: u32,
}

#[derive(Debug)]
enum Shape {
    Brush {
        planes: Vec<[f32; 4]>,
        surfaces: Vec<u32>,
    },
    Triangle {
        vertices: [V3; 3],
    },
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
    pub captured_contents_mask: u32,
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
    pub contents: u32,
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            fraction: 1.0,
            normal: [0.0; 3],
            start_solid: false,
            all_solid: false,
            surface_flags: 0,
            contents: 0,
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
            ensure!(
                b.plane_surface_flags.is_empty()
                    || b.plane_surface_flags.len() == b.planes.len() + 6,
                "brush surface count does not match its planes"
            );
            let surfaces = if b.plane_surface_flags.is_empty() {
                vec![0; b.planes.len() + 6]
            } else {
                b.plane_surface_flags
            };
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
            if b.contents & (PLAYER_MASK | SHOT_MASK) != 0 {
                primitives.push(Primitive {
                    bounds,
                    contents: b.contents,
                    surface_flags: 0,
                    shape: Shape::Brush { planes, surfaces },
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
            if length(face) < 0.5 || t.contents & (PLAYER_MASK | SHOT_MASK) == 0 {
                continue;
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
                },
            });
        }
        ensure!(
            !primitives.is_empty(),
            "collision export contains no player- or shot-solid geometry"
        );
        let mut collision = Self {
            map: export.map,
            ncs_crc: export.ncs_crc,
            captured_contents_mask: export.captured_contents_mask,
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
    /// Sweep an upright player capsule whose origin is at its feet.
    pub fn trace(&self, start: V3, end: V3, height: f32) -> Trace {
        let capsule = Capsule::new(height);
        let half = [capsule.radius, capsule.radius, height * 0.5];
        let offset = [0.0, 0.0, half[2]];
        let start = add(start, offset);
        let end = add(end, offset);
        let expanded = add(half, [SKIN; 3]);
        let bounds = Bounds::center_half(start, expanded).union(Bounds::center_half(end, expanded));
        let mut result = Trace::default();
        self.trace_node(
            0,
            bounds,
            start,
            end,
            Sweep {
                capsule,
                mask: PLAYER_MASK,
            },
            &mut result,
        );
        result
    }
    /// Cast a zero-width segment through exported world geometry. Unlike the
    /// player sweep, this applies neither the capsule radius nor movement skin.
    /// A point beginning inside a solid brush reports fraction zero even when
    /// its endpoint escapes, so a shot cannot originate through that brush.
    pub fn raycast(&self, start: V3, end: V3) -> Trace {
        self.raycast_mask(start, end, SHOT_MASK)
    }
    pub(crate) fn projectile_trace(&self, start: V3, end: V3, radius: f32) -> Trace {
        let capsule = Capsule {
            radius,
            segment_half: 0.0,
        };
        let half = [radius; 3];
        let bounds = Bounds::center_half(start, half).union(Bounds::center_half(end, half));
        let mut trace = Trace::default();
        self.trace_node(
            0,
            bounds,
            start,
            end,
            Sweep {
                capsule,
                mask: SHOT_MASK,
            },
            &mut trace,
        );
        trace
    }
    pub fn movement_surface(&self, start: V3, end: V3) -> Trace {
        self.raycast_mask(start, end, PLAYER_MASK)
    }
    fn raycast_mask(&self, start: V3, end: V3, mask: u32) -> Trace {
        let bounds = Bounds {
            min: std::array::from_fn(|i| start[i].min(end[i])),
            max: std::array::from_fn(|i| start[i].max(end[i])),
        };
        let mut result = Trace::default();
        self.raycast_node(0, bounds, start, end, mask, &mut result);
        result
    }
    fn raycast_node(
        &self,
        index: usize,
        bounds: Bounds,
        start: V3,
        end: V3,
        mask: u32,
        result: &mut Trace,
    ) {
        if !self.nodes[index].bounds().intersects(bounds) {
            return;
        }
        match self.nodes[index] {
            Node::Branch { left, right, .. } => {
                self.raycast_node(left, bounds, start, end, mask, result);
                self.raycast_node(right, bounds, start, end, mask, result);
            }
            Node::Leaf { first, count, .. } => {
                for &i in &self.indices[first..first + count] {
                    let primitive = &self.primitives[i];
                    if primitive.contents & mask == 0 || !primitive.bounds.intersects(bounds) {
                        continue;
                    }
                    let hit = match &primitive.shape {
                        Shape::Brush { planes, surfaces } => {
                            ray_brush(planes, surfaces, start, end)
                        }
                        Shape::Triangle { vertices } => ray_triangle(*vertices, start, end),
                    };
                    let Some(hit) = hit else { continue };
                    let already_start_solid = result.start_solid;
                    result.start_solid |= hit.start_solid;
                    result.all_solid |= hit.all_solid;
                    if hit.fraction < result.fraction
                        || (hit.fraction == result.fraction
                            && (result.contents == 0 || (hit.start_solid && !already_start_solid)))
                    {
                        result.fraction = hit.fraction;
                        result.normal = hit.normal;
                        result.surface_flags = match primitive.shape {
                            Shape::Brush { .. } => hit.surface_flags,
                            _ => primitive.surface_flags,
                        };
                        result.contents = primitive.contents;
                    }
                }
            }
        }
    }
    fn trace_node(
        &self,
        index: usize,
        bounds: Bounds,
        start: V3,
        end: V3,
        sweep: Sweep,
        result: &mut Trace,
    ) {
        if !self.nodes[index].bounds().intersects(bounds) {
            return;
        }
        match self.nodes[index] {
            Node::Branch { left, right, .. } => {
                self.trace_node(left, bounds, start, end, sweep, result);
                self.trace_node(right, bounds, start, end, sweep, result);
            }
            Node::Leaf { first, count, .. } => {
                for &i in &self.indices[first..first + count] {
                    let p = &self.primitives[i];
                    if p.contents & sweep.mask == 0 || !p.bounds.intersects(bounds) {
                        continue;
                    }
                    let hit = match &p.shape {
                        Shape::Brush { planes, surfaces } => {
                            sweep_brush(planes, surfaces, start, end, sweep.capsule)
                        }
                        Shape::Triangle { vertices } => {
                            sweep_triangle(*vertices, start, end, sweep.capsule)
                        }
                    };
                    result.start_solid |= hit.start_solid;
                    result.all_solid |= hit.all_solid;
                    if hit.fraction < result.fraction {
                        result.fraction = hit.fraction;
                        result.normal = hit.normal;
                        result.surface_flags = match p.shape {
                            Shape::Brush { .. } => hit.surface_flags,
                            _ => p.surface_flags,
                        };
                        result.contents = p.contents;
                    }
                }
            }
        }
    }
}

/// Exact point-versus-convex-brush interval clipping. Movement skin must not
/// move a bullet's reported intersection away from the actual world surface.
fn ray_brush(planes: &[[f32; 4]], surfaces: &[u32], start: V3, end: V3) -> Option<Trace> {
    let mut enter = 0.0f64;
    let mut leave = 1.0f64;
    let mut normal = [0.0; 3];
    let mut surface_flags = 0;
    let mut start_inside = true;
    let mut end_inside = true;
    for (index, plane) in planes.iter().enumerate() {
        let n = [plane[0], plane[1], plane[2]];
        let distance = |point: V3| {
            (0..3)
                .map(|i| f64::from(n[i]) * f64::from(point[i]))
                .sum::<f64>()
                - f64::from(plane[3])
        };
        let first = distance(start);
        let second = distance(end);
        start_inside &= first < 0.0;
        end_inside &= second <= 0.0;
        if first > 0.0 && second > 0.0 {
            return None;
        }
        if first <= 0.0 && second <= 0.0 {
            if first == 0.0 && length(normal) == 0.0 {
                normal = normalized(n);
                surface_flags = surfaces[index];
            }
            continue;
        }
        let fraction = first / (first - second);
        if first > second {
            if fraction >= enter {
                enter = fraction;
                normal = normalized(n);
                surface_flags = surfaces[index];
            }
        } else {
            leave = leave.min(fraction);
        }
        if enter > leave {
            return None;
        }
    }
    Some(Trace {
        fraction: if start_inside { 0.0 } else { enter as f32 },
        normal: if start_inside { [0.0; 3] } else { normal },
        start_solid: start_inside,
        all_solid: start_inside && end_inside,
        surface_flags,
        ..Trace::default()
    })
}

/// Double-sided segment/triangle intersection, including boundary vertices,
/// edges and coplanar segments. Intermediate arithmetic uses f64 so a long
/// hitscan segment does not lose small barycentric distances near a map edge.
fn ray_triangle(vertices: [V3; 3], start: V3, end: V3) -> Option<Trace> {
    let difference =
        |a: V3, b: V3| -> [f64; 3] { std::array::from_fn(|i| f64::from(a[i]) - f64::from(b[i])) };
    let dot64 = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross64 = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let first_edge = difference(vertices[1], vertices[0]);
    let second_edge = difference(vertices[2], vertices[0]);
    let start_relative = difference(start, vertices[0]);
    let delta = difference(end, start);
    let face = cross64(first_edge, second_edge);
    let area_squared = dot64(face, face);
    if area_squared == 0.0 {
        return None;
    }
    let denominator = dot64(face, delta);
    let numerator = -dot64(face, start_relative);
    let edges = [
        ([0.0; 3], first_edge),
        (
            first_edge,
            std::array::from_fn(|i| second_edge[i] - first_edge[i]),
        ),
        (second_edge, second_edge.map(|x| -x)),
    ];
    let edge_tolerance = area_squared * 1e-12;
    let parallel_tolerance = (area_squared * dot64(delta, delta)).sqrt() * 1e-12;
    let fraction;
    if denominator.abs() > parallel_tolerance {
        let intersection = numerator / denominator;
        if !(-1e-12..=1.0 + 1e-12).contains(&intersection) {
            return None;
        }
        let point: [f64; 3] = std::array::from_fn(|i| start_relative[i] + delta[i] * intersection);
        for (origin, edge) in edges {
            let relative = std::array::from_fn(|i| point[i] - origin[i]);
            if dot64(cross64(edge, relative), face) < -edge_tolerance {
                return None;
            }
        }
        fraction = intersection.clamp(0.0, 1.0);
    } else {
        // Parallel segments can hit only when they lie in the triangle's
        // plane. Clip their interval against its three oriented edge planes.
        if numerator.abs() > area_squared.sqrt() * 1e-9 {
            return None;
        }
        let mut enter = 0.0f64;
        let mut leave = 1.0f64;
        for (origin, edge) in edges {
            let relative = std::array::from_fn(|i| start_relative[i] - origin[i]);
            let distance = dot64(cross64(edge, relative), face);
            let speed = dot64(cross64(edge, delta), face);
            if speed.abs() <= edge_tolerance {
                if distance < -edge_tolerance {
                    return None;
                }
            } else if speed > 0.0 {
                enter = enter.max(-distance / speed);
            } else {
                leave = leave.min(-distance / speed);
            }
            if enter > leave + 1e-12 {
                return None;
            }
        }
        fraction = enter.clamp(0.0, 1.0);
    }
    let mut normal = normalized(face.map(|x| x as f32));
    if denominator > 0.0 {
        normal = mul(normal, -1.0);
    }
    // A terrain triangle has no volume: touching it at the start is a hit,
    // but is not reported as an interior/all-solid brush penetration.
    Some(Trace {
        fraction: fraction as f32,
        normal,
        ..Trace::default()
    })
}

#[derive(Clone, Copy)]
struct Capsule {
    radius: f32,
    segment_half: f32,
}
#[derive(Clone, Copy)]
struct Sweep {
    capsule: Capsule,
    mask: u32,
}
impl Capsule {
    fn new(height: f32) -> Self {
        let radius = 15.0f32.min(height * 0.5);
        Self {
            radius,
            segment_half: height * 0.5 - radius,
        }
    }
    fn support(self, normal: V3) -> f32 {
        self.radius * length(normal) + self.segment_half * normal[2].abs()
    }
    fn segment(self, center: V3) -> [V3; 2] {
        let offset = [0.0, 0.0, self.segment_half];
        [sub(center, offset), add(center, offset)]
    }
}

fn sweep_brush(
    planes: &[[f32; 4]],
    surfaces: &[u32],
    start: V3,
    end: V3,
    capsule: Capsule,
) -> Trace {
    let mut enter = -f32::INFINITY;
    let mut leave = 1.0f32;
    let mut normal = [0.0; 3];
    let mut surface_flags = 0;
    let mut outside = false;
    let mut end_out = false;
    for (index, p) in planes.iter().enumerate() {
        let n = [p[0], p[1], p[2]];
        let radius = capsule.support(n);
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
                surface_flags = surfaces.get(index).copied().unwrap_or(0);
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
            surface_flags,
            ..Trace::default()
        }
    } else {
        Trace::default()
    }
}

/// Vector from the nearest point on the triangle to `point`. The interior
/// region uses the plane normal directly, avoiding rounded barycentric points
/// that otherwise perturb tangent motion on large terrain triangles.
fn point_triangle_separation(point: V3, vertices: [V3; 3]) -> V3 {
    let [a, b, c] = vertices;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(point, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap;
    }
    let bp = sub(point, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return bp;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return sub(ap, mul(ab, d1 / (d1 - d3)));
    }
    let cp = sub(point, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return cp;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return sub(ap, mul(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return sub(bp, mul(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let normal = normalized(cross(ab, ac));
    mul(normal, dot(ap, normal))
}

/// Closest points between two finite segments, including a zero-length
/// segment (the prone capsule is a sphere).
fn segment_segment_separation(a: V3, b: V3, c: V3, d: V3) -> V3 {
    let first = sub(b, a);
    let second = sub(d, c);
    let offset = sub(a, c);
    let aa = dot(first, first);
    let bb = dot(first, second);
    let cc = dot(second, second);
    let dd = dot(first, offset);
    let ee = dot(second, offset);
    let (mut s, mut t);
    if aa <= 1e-8 {
        s = 0.0;
        t = if cc <= 1e-8 {
            0.0
        } else {
            (ee / cc).clamp(0.0, 1.0)
        };
    } else if cc <= 1e-8 {
        t = 0.0;
        s = (-dd / aa).clamp(0.0, 1.0);
    } else {
        let denominator = aa * cc - bb * bb;
        s = if denominator > aa * cc * 1e-7 {
            ((bb * ee - cc * dd) / denominator).clamp(0.0, 1.0)
        } else {
            0.0
        };
        t = (bb * s + ee) / cc;
        if t < 0.0 {
            t = 0.0;
            s = (-dd / aa).clamp(0.0, 1.0);
        } else if t > 1.0 {
            t = 1.0;
            s = ((bb - dd) / aa).clamp(0.0, 1.0);
        }
    }
    // Work in relative coordinates to preserve small contact distances.
    sub(add(offset, mul(first, s)), mul(second, t))
}

/// Minimum segment-to-triangle separation: segment endpoints versus the
/// triangle, segment versus each edge, and an interior face intersection.
/// These features cover the cylindrical body as well as both spherical caps.
fn capsule_triangle_separation(capsule: Capsule, center: V3, vertices: [V3; 3]) -> V3 {
    let [a, b] = capsule.segment(center);
    let normal = normalized(cross(
        sub(vertices[1], vertices[0]),
        sub(vertices[2], vertices[0]),
    ));
    let start_distance = dot(sub(a, vertices[0]), normal);
    let end_distance = dot(sub(b, vertices[0]), normal);
    if start_distance * end_distance <= 0.0 && (start_distance - end_distance).abs() > 1e-8 {
        let point = add(
            a,
            mul(sub(b, a), start_distance / (start_distance - end_distance)),
        );
        if length(point_triangle_separation(point, vertices)) < 0.001 {
            return [0.0; 3];
        }
    }
    let mut best = point_triangle_separation(a, vertices);
    let other = point_triangle_separation(b, vertices);
    if dot(other, other) < dot(best, best) {
        best = other;
    }
    for i in 0..3 {
        let other = segment_segment_separation(a, b, vertices[i], vertices[(i + 1) % 3]);
        if dot(other, other) < dot(best, best) {
            best = other;
        }
    }
    best
}

/// Continuous capsule sweep using the exact segment-to-triangle distance.
/// The closest-feature plane is a separating plane, so advancing by its gap
/// divided by closing speed cannot skip an earlier edge/vertex collision.
fn sweep_triangle(vertices: [V3; 3], start: V3, end: V3, capsule: Capsule) -> Trace {
    const DISTANCE_TOLERANCE: f32 = 0.0005;
    let delta = sub(end, start);
    let separation = |fraction: f32| {
        capsule_triangle_separation(capsule, add(start, mul(delta, fraction)), vertices)
    };
    let start_distance = length(separation(0.0));
    if start_distance < capsule.radius - 0.001 {
        let all_solid = length(separation(1.0)) < capsule.radius - 0.001;
        return Trace {
            fraction: if all_solid { 0.0 } else { 1.0 },
            start_solid: true,
            all_solid,
            ..Trace::default()
        };
    }
    let radius = capsule.radius + SKIN;
    let mut fraction = 0.0;
    for _ in 0..32 {
        let difference = separation(fraction);
        let distance = length(difference);
        let normal = mul(difference, 1.0 / distance.max(1e-8));
        let closing = -dot(delta, normal);
        // Distance along a line to this convex shape is convex. If it is
        // stationary or increasing here, it cannot enter later in this sweep.
        if closing <= 1e-8 {
            return Trace::default();
        }
        let gap = distance - radius;
        if gap <= DISTANCE_TOLERANCE {
            return Trace {
                fraction,
                normal,
                ..Trace::default()
            };
        }
        let next = fraction + gap / closing;
        if next > 1.0 {
            return Trace::default();
        }
        if next <= fraction {
            break;
        }
        fraction = next;
    }
    // Grazing sweeps may converge very slowly. Locate the minimum of the
    // convex distance, then bisect its entering root; do not drop the hit
    // simply because conservative advancement exhausted its iteration budget.
    let mut left = fraction;
    let mut right = 1.0;
    for _ in 0..48 {
        let first = left + (right - left) / 3.0;
        let second = right - (right - left) / 3.0;
        if length(separation(first)) <= length(separation(second)) {
            right = second;
        } else {
            left = first;
        }
    }
    let minimum = (left + right) * 0.5;
    if length(separation(minimum)) > radius + DISTANCE_TOLERANCE {
        return Trace::default();
    }
    left = fraction;
    right = minimum;
    for _ in 0..24 {
        let middle = (left + right) * 0.5;
        if length(separation(middle)) <= radius + DISTANCE_TOLERANCE {
            right = middle;
        } else {
            left = middle;
        }
    }
    Trace {
        fraction: right,
        normal: normalized(separation(right)),
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
    pub sprint: codol_protocol::game_snapshot::SprintState,
    sprint_rules: crate::game_sprint::Rules,
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
    melee_charge: Option<(codol_protocol::game_snapshot::MeleeCharge, u16)>,
    previous_buttons: u32,
    ground_normal: V3,
    view_start: f32,
}
impl Motion {
    pub fn new(origin: V3) -> Self {
        Self {
            sprint: Default::default(),
            sprint_rules: Default::default(),
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
            melee_charge: None,
            previous_buttons: 0,
            ground_normal: [0.0, 0.0, 1.0],
            view_start: 60.0,
        }
    }
    pub fn grounded(&self) -> bool {
        self.ground_entity != NONE
    }
    pub(crate) fn set_sprint_rules(&mut self, rules: crate::game_sprint::Rules) {
        self.sprint_rules = rules;
    }
    pub(crate) fn cancel_sprint(&mut self) {
        if self.flags & 0x4000 != 0 {
            crate::game_sprint::stop(
                &mut self.sprint,
                self.command_time,
                self.previous_buttons & 2 != 0,
            );
            self.flags &= !0x4000;
        }
    }
    pub fn melee_charge(&self) -> Option<codol_protocol::game_snapshot::MeleeCharge> {
        self.melee_charge.map(|(state, _)| state)
    }
    pub(crate) fn start_melee_charge(
        &mut self,
        approach: crate::game_player::MeleeApproach,
        duration: u16,
    ) {
        if approach.distance == 0 || !(1..=15000).contains(&duration) || self.melee_charge.is_some()
        {
            return;
        }
        self.melee_charge = Some((
            codol_protocol::game_snapshot::MeleeCharge {
                yaw: approach.yaw,
                distance: approach.distance,
                remaining_msec: 0,
            },
            duration,
        ));
        self.cancel_sprint();
        self.flags |= 0x10000;
    }
    pub fn cancel_melee_charge(&mut self) {
        self.melee_charge = None;
        self.flags &= !0x10000;
    }
    fn step_melee_charge(&mut self, dt_ms: u32, c: &Collision) {
        let (mut state, duration) = self.melee_charge.unwrap();
        if state.remaining_msec == 0 {
            let seconds = (f64::from(duration) * 0.001) as f32;
            let speed = 2.0 * f32::from(state.distance) / seconds;
            let yaw = f32::from(state.yaw as i16) * (std::f32::consts::TAU / 65536.0);
            self.velocity[0] = speed * yaw.cos();
            self.velocity[1] = speed * yaw.sin();
            // 0x41cbaa stores seconds as binary32, multiplies by the double
            //1000 constant and truncates when initializing the timer.
            state.remaining_msec = (f64::from(seconds) * 1000.0) as u16;
        }
        let speed = length(self.velocity);
        let direction = normalized(self.velocity);
        let dt = (f64::from(dt_ms) * 0.001) as f32;
        let next = (speed - speed / (f32::from(state.remaining_msec) * 0.001) * dt).max(0.0);
        self.velocity = mul(direction, (speed + next) * 0.5);
        if self.grounded() {
            self.velocity = project_ground(self.velocity, self.ground_normal);
        }
        if self.velocity[0] != 0.0 || self.velocity[1] != 0.0 {
            self.step_slide(c, dt, false);
        }
        self.velocity = mul(direction, next);
        state.remaining_msec = state.remaining_msec.saturating_sub(dt_ms as u16);
        self.flags = self.stance.flags() | 0x10000 | (self.flags & 0x8000);
        if state.remaining_msec == 0 {
            self.velocity[0] = 0.0;
            self.velocity[1] = 0.0;
            self.cancel_melee_charge();
        } else {
            self.melee_charge = Some((state, duration));
        }
        self.ground(c, 0.25);
    }
    /// Apply a small crowd-avoidance displacement through the same capsule
    /// solver as walking. This is not elapsed movement time: preserve the
    /// actor's velocity and clocks, and never apply gravity a second time.
    pub fn move_separation(&mut self, shift: [f32; 2], c: &Collision) {
        if shift == [0.0; 2] {
            return;
        }
        self.ground(c, 0.25);
        let velocity = self.velocity;
        self.velocity = [shift[0], shift[1], 0.0];
        if self.grounded() {
            self.velocity[2] = -(shift[0] * self.ground_normal[0]
                + shift[1] * self.ground_normal[1])
                / self.ground_normal[2];
            self.step_slide(c, 1.0, false);
        } else {
            // Airborne avoidance may slide against a wall but cannot step up
            // or attach itself to a floor below a ledge.
            self.slide(c, 1.0, false);
        }
        self.velocity = velocity;
        self.ground(c, 0.25);
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
        if self.melee_charge.is_some() {
            self.step_melee_charge(dt_ms, c);
            return;
        }
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
        let sprint = crate::game_sprint::update(
            &mut self.sprint,
            self.command_time,
            cmd.buttons,
            self.previous_buttons,
            self.flags & 0x4000 != 0,
            f <= 105.0
                || ads
                || self.stance != Stance::Stand
                || self.flags & (0x8000 | 0x20000) != 0,
            self.sprint_rules,
        );
        self.flags = self.stance.flags()
            | (self.flags & 0x8000)
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
                self.velocity = [0.0; 3];
            } else {
                let scale = ((horizontal - horizontal.max(100.0) * 5.5 * dt).max(0.0)) / horizontal;
                // Target PM_Friction 0x416390 measures horizontal speed on
                // ground but applies the resulting scale to all three axes.
                self.velocity = mul(self.velocity, scale);
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
        // Target WalkMove skips step-slide when horizontal velocity is zero.
        // In particular a ground-classification probe must not move an idle
        // player's origin or project its velocity on every input command.
        if !was_grounded || self.velocity[0] != 0.0 || self.velocity[1] != 0.0 {
            self.step_slide(c, dt, !was_grounded);
        }
        self.ground(c, 0.25);
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
        // PM_GroundTrace 0x418500 probes +0.25 -> -0.25. Position changes
        // belong to StepSlideMove; the probe only classifies the current hull.
        let trace = c.trace(
            add(self.origin, [0.0, 0.0, distance]),
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
            self.ground_entity = WORLD;
            self.ground_normal = trace.normal;
            self.surface_flags = trace.surface_flags;
            self.flags &= !0x2000;
        } else {
            self.ground_entity = NONE;
        }
    }
    fn step_slide(&mut self, c: &Collision, dt: f32, gravity: bool) {
        let start = self.origin;
        let velocity = self.velocity;
        let on_ground = self.grounded();
        let blocked = self.slide(c, dt, gravity);
        let down_origin = self.origin;
        let down_velocity = self.velocity;
        if gravity {
            // Slide already swept the complete time interval. Sweeping another
            // velocity*dt here would land on a floor only reached next frame.
            // GroundTrace's 0.25-unit classification includes the sweep skin.
            return;
        }
        let height = if self.stance == Stance::Prone {
            10.0
        } else {
            18.0
        };
        let mut step_up = 0.0;
        // Target 0x421680 attempts a step on a blocked move or a steep
        // ground plane, with a one-unit headroom margin.
        if blocked || (on_ground && self.ground_normal[2] < 0.9) {
            let up = c.trace(
                start,
                add(start, [0.0, 0.0, height + 1.0]),
                self.stance.height(),
            );
            step_up = up.fraction * (height + 1.0) - 1.0;
            if up.start_solid || step_up < 1.0 {
                step_up = 0.0;
            } else {
                self.origin = add(start, [0.0, 0.0, step_up]);
                self.velocity = velocity;
                self.slide(c, dt, gravity);
            }
        }
        if on_ground || step_up != 0.0 {
            let distance = step_up + if on_ground { 9.0 } else { 0.0 };
            let down = c.trace(
                self.origin,
                add(self.origin, [0.0, 0.0, -distance]),
                self.stance.height(),
            );
            if down.start_solid || (down.fraction < 1.0 && down.normal[2] < 0.3) {
                self.origin = down_origin;
                self.velocity = down_velocity;
                return;
            }
            if down.fraction == 1.0 {
                self.origin[2] -= step_up;
            } else {
                self.origin[2] -= distance * down.fraction;
                self.velocity = project_ground(self.velocity, down.normal);
            }
        }
        // Compare progress along the initial velocity, not squared distance:
        // a sideways deflection must not win over a useful forward move.
        let slide_dot =
            (down_origin[0] - start[0]) * velocity[0] + (down_origin[1] - start[1]) * velocity[1];
        let step_dot =
            (self.origin[0] - start[0]) * velocity[0] + (self.origin[1] - start[1]) * velocity[1];
        if slide_dot + 0.001 >= step_dot {
            self.origin = down_origin;
            self.velocity = down_velocity;
            if on_ground {
                let down = c.trace(
                    self.origin,
                    add(self.origin, [0.0, 0.0, -9.0]),
                    self.stance.height(),
                );
                if down.fraction < 1.0 && !down.start_solid {
                    self.origin[2] -= 9.0 * down.fraction;
                    self.velocity = clip(self.velocity, down.normal);
                }
            }
        }
        let dz = self.origin[2] - down_origin[2];
        if on_ground && dz.abs() > 0.5 && dz.round() != 0.0 {
            // The client slows a real step. Omitting this produces recurring
            // prediction corrections when climbing stairs.
            let scale = (1.0 - (self.origin[2] - start[2]).abs() / height) * 0.8 + 0.2;
            self.velocity = mul(self.velocity, scale);
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
    // Target subtracts abs(dot)*0.001; dividing a positive dot by 1.001 is
    // close but is a different operation after repeated ground clips.
    sub(v, mul(n, d - d.abs() * 0.001))
}
fn project_ground(v: V3, n: V3) -> V3 {
    let horizontal_squared = v[0] * v[0] + v[1] * v[1];
    if n[2].abs() < 0.001 || horizontal_squared == 0.0 {
        return v;
    }
    let z = -(v[0] * n[0] + v[1] * n[1]) / n[2];
    let result = [v[0], v[1], z];
    let scale = ((v[2] * v[2] + horizontal_squared) / (z * z + horizontal_squared)).sqrt();
    if scale < 1.0 || z < 0.0 || v[2] > 0.0 {
        mul(result, scale)
    } else {
        v
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
            captured_contents_mask: PLAYER_MASK | SHOT_MASK,
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
            plane_surface_flags: vec![],
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
    fn marathon_changes_real_movement_after_the_normal_meter_runs_out() {
        let c = world(vec![floor()], vec![]);
        let mut normal = Motion::new([0.0, 0.0, 0.03125]);
        normal.apply(&cmd(1000, 0, 0), 1.0, false, &c);
        let mut marathon = normal.clone();
        marathon.set_sprint_rules(crate::game_sprint::Rules {
            duration_seconds: -1.0,
            ..Default::default()
        });
        for time in (1016..=6616).step_by(16) {
            let input = cmd(time, 1, 2);
            normal.apply(&input, 1.0, false, &c);
            marathon.apply(&input, 1.0, false, &c);
        }
        assert_eq!(normal.flags & 0x4000, 0);
        assert!(normal.sprint.button_up_required && normal.sprint.delay);
        assert_ne!(marathon.flags & 0x4000, 0);
        assert!(marathon.origin[0] > normal.origin[0] + 100.0);
        let before = normal.sprint;
        normal.apply(&cmd(normal.command_time, 1, 0), 1.0, false, &c);
        assert_eq!(normal.sprint, before);
        normal.apply(&cmd(6632, 1, 0), 1.0, false, &c);
        normal.apply(&cmd(6648, 1, 2), 1.0, false, &c);
        assert_ne!(normal.flags & 0x4000, 0);
        normal.cancel_sprint();
        assert_eq!(normal.flags & 0x4000, 0);
        assert_eq!(normal.sprint.last_end, normal.command_time);
        let stopped = normal.sprint;
        normal.cancel_sprint();
        assert_eq!(normal.sprint, stopped);
    }
    #[test]
    fn separation_respects_walls_and_preserves_airborne_velocity_and_time() {
        let c = world(
            vec![floor(), brush([50.0, 0.0, 100.0], [10.0, 1000.0, 100.0])],
            vec![],
        );
        let mut m = Motion::new([0.0, 0.0, 0.03125]);
        tick(&mut m, &c, 0, 0, 1);
        let time = m.command_time;
        for _ in 0..20 {
            m.move_separation([4.0, 0.0], &c);
            assert!(m.grounded());
            assert!(!c.trace(m.origin, m.origin, 70.0).start_solid);
        }
        assert!(m.origin[0] > 24.0 && m.origin[0] < 25.0);
        assert_eq!(m.command_time, time);
        for vertical in [-50.0, 50.0] {
            let mut air = Motion::new([0.0, 0.0, 5.0]);
            air.velocity = [10.0, 20.0, vertical];
            air.move_separation([4.0, 0.0], &c);
            assert_eq!(air.origin, [4.0, 0.0, 5.0]);
            assert_eq!(air.velocity, [10.0, 20.0, vertical]);
            assert_eq!(air.command_time, 0);
            assert!(!air.grounded());
        }
    }
    #[test]
    fn melee_charge_uses_native_midpoint_deceleration_and_clears_at_expiry() {
        let c = world(vec![floor()], vec![]);
        let mut m = Motion::new([0.0, 0.0, 0.03125]);
        m.apply(&cmd(1000, 0, 0), 1.0, false, &c);
        m.start_melee_charge(
            crate::game_player::MeleeApproach {
                yaw: 0,
                distance: 50,
            },
            250,
        );
        assert_eq!(m.origin[0], 0.0);
        assert_eq!(m.melee_charge().unwrap().remaining_msec, 0);
        for (i, x) in [18.0, 32.0, 42.0, 48.0, 50.0].into_iter().enumerate() {
            let input = cmd(1050 + i as u32 * 50, 0, 0);
            m.apply(&input, 1.0, false, &c);
            assert!((m.origin[0] - x).abs() < 0.002, "{:?}", m.origin);
            let before = m.origin;
            m.apply(&input, 1.0, false, &c);
            assert_eq!(m.origin, before);
        }
        assert!(m.melee_charge().is_none());
        assert_eq!(m.flags & 0x10000, 0);
        assert_eq!(m.velocity, [0.0; 3]);
    }
    #[test]
    fn melee_charge_matches_native_expiry_during_a_long_command() {
        let c = world(vec![floor()], vec![]);
        let mut m = Motion::new([0.0, 0.0, 0.03125]);
        m.apply(&cmd(1509, 0, 0), 1.0, false, &c);
        m.start_melee_charge(
            crate::game_player::MeleeApproach {
                yaw: 466,
                distance: 75,
            },
            100,
        );
        m.apply(&cmd(1739, 0, 0), 1.0, false, &c);
        // Native command1509->1739, client-1791694333310: the last
        //66ms subdivision spans timer expiry and still integrates its full dt.
        assert!((m.origin[0] - 83.07715).abs() < 0.002, "{:?}", m.origin);
        assert!((m.origin[1] - 3.7141).abs() < 0.002, "{:?}", m.origin);
        assert!(m.melee_charge().is_none());
        assert_eq!(m.velocity, [0.0; 3]);
    }
    #[test]
    fn melee_charge_sweeps_the_capsule_and_cannot_cross_a_wall() {
        let c = world(
            vec![floor(), brush([60.0, 0.0, 100.0], [5.0, 1000.0, 100.0])],
            vec![],
        );
        let mut m = Motion::new([0.0, 0.0, 0.03125]);
        m.apply(&cmd(1000, 0, 0), 1.0, false, &c);
        m.start_melee_charge(
            crate::game_player::MeleeApproach {
                yaw: 0,
                distance: 200,
            },
            100,
        );
        for time in (1010..=1100).step_by(10) {
            m.apply(&cmd(time, 0, 0), 1.0, false, &c);
        }
        assert!(m.origin[0] > 39.0 && m.origin[0] < 40.0, "{:?}", m.origin);
        assert!(!c.trace(m.origin, m.origin, 70.0).start_solid);
        assert!(m.melee_charge().is_none());
        assert_eq!(m.velocity, [0.0; 3]);
    }
    #[test]
    fn separation_leaves_a_ledge_without_snapping_to_a_distant_floor() {
        let c = world(
            vec![
                brush([0.0, 0.0, -20.0], [32.0, 1000.0, 20.0]),
                brush([0.0, 0.0, -120.0], [1000.0, 1000.0, 20.0]),
            ],
            vec![],
        );
        let mut m = Motion::new([30.0, 0.0, 0.03125]);
        tick(&mut m, &c, 0, 0, 1);
        for _ in 0..10 {
            m.move_separation([4.0, 0.0], &c);
        }
        assert!(!m.grounded());
        assert!(m.origin[0] > 60.0 && m.origin[2] > -18.0, "{:?}", m.origin);
        tick(&mut m, &c, 0, 0, 100);
        assert!(m.grounded());
        assert!((m.origin[2] + 100.0).abs() < 0.1);
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
    fn falling_only_advances_the_command_interval_before_landing() {
        let c = world(vec![floor()], vec![]);
        let mut m = Motion::new([0.0, 0.0, 5.0]);
        m.command_time = 1000;
        m.velocity = [0.0, 0.0, -40.0];
        m.apply(&cmd(1050, 0, 0), 1.0, false, &c);
        assert!((m.origin[2] - 2.0).abs() < 0.001, "{:?}", m);
        assert_eq!(m.velocity[2], -80.0);
        assert!(!m.grounded());
        m.apply(&cmd(1100, 0, 0), 1.0, false, &c);
        assert!((0.0..=0.25).contains(&m.origin[2]), "{:?}", m);
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
        // GroundTrace classifies contact within 0.25 units without snapping
        // the origin. Check that actual tolerance, rather than requiring the
        // old second integration to force exactly the previous resting point.
        assert!((0.0..=0.25).contains(&m.origin[2]), "{m:?}");
        assert!(
            (m.origin[2] - base).abs() <= 0.25,
            "base={base}, motion={m:?}"
        );
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
        let slope_offset = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0);
        assert!(
            (m.origin[2] - m.origin[0] * 0.25 - slope_offset).abs() < 0.1,
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
        let slope_offset = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0);
        let mut m = Motion::new([0.0, 0.0, slope_offset + 0.05]);
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
    #[test]
    fn capsule_support_uses_a_round_cap_instead_of_box_corners() {
        let normal = normalized([-0.25, 0.0, 1.0]);
        for height in [70.0, 50.0, 30.0] {
            let capsule = Capsule::new(height);
            let expected = 15.0 + (height * 0.5 - 15.0) * normal[2];
            assert!((capsule.support(normal) - expected).abs() < 0.00001);
            let foot_offset = capsule.support(normal) / normal[2] - height * 0.5;
            let expected_offset = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0);
            assert!((foot_offset - expected_offset).abs() < 0.00001);
        }
    }
    #[test]
    fn capsule_brush_slope_uses_the_same_round_cap_as_terrain() {
        let normal = normalized([-0.25, 0.0, 1.0]);
        let planes = [[normal[0], normal[1], normal[2], 0.0]];
        let capsule = Capsule::new(70.0);
        let trace = sweep_brush(&planes, &[], [0.0, 0.0, 45.0], [0.0, 0.0, 25.0], capsule);
        let foot = 10.0 - 20.0 * trace.fraction;
        let expected = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0) + SKIN / normal[2];
        assert!((foot - expected).abs() < 0.0001, "{trace:?}, foot={foot}");
    }
    #[test]
    fn capsule_terrain_sweep_covers_edges_vertices_and_cylinder() {
        let vertices = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
        let capsule = Capsule::new(70.0);
        let edge = sweep_triangle(vertices, [50.0, -100.0, 20.0], [50.0, 100.0, 20.0], capsule);
        assert!(
            (edge.fraction - (100.0 - 15.0 - SKIN) / 200.0).abs() < 0.00001,
            "{edge:?}"
        );
        assert!(edge.normal[1] < -0.999, "{edge:?}");
        let vertex = sweep_triangle(
            vertices,
            [-100.0, -100.0, 20.0],
            [100.0, 100.0, 20.0],
            capsule,
        );
        let expected = (100.0 - (15.0 + SKIN) / 2.0f32.sqrt()) / 200.0;
        assert!((vertex.fraction - expected).abs() < 0.00001, "{vertex:?}");
        assert!(
            vertex.normal[0] < -0.7 && vertex.normal[1] < -0.7,
            "{vertex:?}"
        );
        // A box corner overlaps this triangle, but the round capsule does not.
        let clear = sweep_triangle(
            vertices,
            [-14.0, -14.0, 20.0],
            [-14.0, -14.0, 20.0],
            capsule,
        );
        assert!(!clear.start_solid && clear.fraction == 1.0, "{clear:?}");
        // Contact halfway along the vertical segment is outside either end
        // sphere, proving that the cylinder is included in position tests.
        let upright = [[0.0, 0.0, 30.0], [0.0, 0.0, 40.0], [0.0, 1.0, 35.0]];
        let center = [-14.0, 0.0, 20.0];
        let solid = sweep_triangle(upright, center, center, capsule);
        assert!(solid.start_solid && solid.all_solid, "{solid:?}");
    }
    #[test]
    fn capsule_terrain_sweep_does_not_tunnel_at_high_speed() {
        let vertices = [
            [-1000.0, -1000.0, 0.0],
            [1000.0, -1000.0, 0.0],
            [0.0, 1000.0, 0.0],
        ];
        let capsule = Capsule::new(70.0);
        let face = sweep_triangle(vertices, [0.0, 0.0, 10035.0], [0.0, 0.0, -9965.0], capsule);
        assert!(
            (face.fraction - (10000.0 - SKIN) / 20000.0).abs() < 0.000001,
            "{face:?}"
        );
        assert!(face.normal[2] > 0.999, "{face:?}");
        let vertices = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
        let edge = sweep_triangle(
            vertices,
            [50.0, -10000.0, 20.0],
            [50.0, 10000.0, 20.0],
            capsule,
        );
        assert!(
            (edge.fraction - (10000.0 - 15.0 - SKIN) / 20000.0).abs() < 0.000001,
            "{edge:?}"
        );
        assert!(edge.normal[1] < -0.999, "{edge:?}");
    }
    #[test]
    fn capsule_does_not_drift_at_rest_or_catch_a_coplanar_terrain_seam() {
        let vertices = [
            [-1000.0, -1000.0, -250.0],
            [1000.0, -1000.0, 250.0],
            [1000.0, 1000.0, 250.0],
            [-1000.0, 1000.0, -250.0],
        ];
        let c = world(
            vec![],
            vec![
                TriangleExport {
                    vertices: [vertices[0], vertices[1], vertices[2]],
                    contents: 1,
                    surface_flags: 0,
                },
                TriangleExport {
                    vertices: [vertices[0], vertices[2], vertices[3]],
                    contents: 1,
                    surface_flags: 0,
                },
            ],
        );
        let mut m = Motion::new([-100.0, -50.0, 100.0]);
        tick(&mut m, &c, 0, 0, 150);
        assert!(m.grounded(), "{m:?}");
        let resting = m.origin;
        tick(&mut m, &c, 0, 0, 600);
        assert_eq!(m.origin, resting, "{m:?}");
        for _ in 0..100 {
            let previous = m.origin;
            tick(&mut m, &c, 1, 0, 1);
            assert!(m.grounded(), "{m:?}");
            assert!(m.origin[0] >= previous[0], "{m:?}");
        }
        assert!(m.origin[0] > 100.0, "{m:?}");
        tick(&mut m, &c, 0, 0, 100);
        let resting = m.origin;
        tick(&mut m, &c, 0, 0, 600);
        assert_eq!(m.origin, resting, "{m:?}");
    }
    #[test]
    fn raycast_returns_the_nearest_world_brush_without_player_radius_or_skin() {
        let mut brushes = (0..12)
            .map(|i| brush([500.0 + i as f32 * 100.0, 0.0, 50.0], [10.0, 10.0, 50.0]))
            .collect::<Vec<_>>();
        brushes.push(brush([100.0, 0.0, 50.0], [10.0, 10.0, 50.0]));
        let c = world(brushes, vec![]);
        let hit = c.raycast([0.0, 0.0, 30.0], [2000.0, 0.0, 30.0]);
        assert!((hit.fraction - 90.0 / 2000.0).abs() < 1e-7, "{hit:?}");
        assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
        assert_eq!(hit.contents, 1);
        assert!(!hit.start_solid && !hit.all_solid);
        let clear = c.raycast([0.0, 11.0, 30.0], [200.0, 11.0, 30.0]);
        assert_eq!(clear.fraction, 1.0, "{clear:?}");
        assert_eq!(clear.contents, 0);
        assert!(c.trace([0.0, 11.0, 0.0], [200.0, 11.0, 0.0], 70.0).fraction < 1.0);
        let short = c.raycast([0.0, 0.0, 30.0], [89.0, 0.0, 30.0]);
        assert_eq!(short.contents, 0, "{short:?}");
        let endpoint = c.raycast([0.0, 0.0, 30.0], [90.0, 0.0, 30.0]);
        assert_eq!(endpoint.fraction, 1.0);
        assert_eq!(endpoint.contents, 1, "{endpoint:?}");
    }
    #[test]
    fn raycast_brush_start_solid_blocks_a_shot_even_when_its_endpoint_escapes() {
        let c = world(vec![brush([0.0; 3], [10.0; 3])], vec![]);
        let escape = c.raycast([0.0; 3], [100.0, 0.0, 0.0]);
        assert_eq!(escape.fraction, 0.0);
        assert!(escape.start_solid && !escape.all_solid, "{escape:?}");
        assert_eq!(escape.contents, 1);
        let inside = c.raycast([0.0; 3], [5.0, 0.0, 0.0]);
        assert!(inside.start_solid && inside.all_solid, "{inside:?}");
        assert_eq!(inside.fraction, 0.0);
        let stationary = c.raycast([0.0; 3], [0.0; 3]);
        assert!(stationary.start_solid && stationary.all_solid);
    }
    #[test]
    fn raycast_terrain_is_double_sided_and_preserves_surface_metadata() {
        let c = world(
            vec![],
            vec![TriangleExport {
                vertices: [[0.0, 0.0, 10.0], [100.0, 0.0, 10.0], [0.0, 100.0, 10.0]],
                contents: 1,
                surface_flags: 0x0050_0100,
            }],
        );
        let front = c.raycast([25.0, 25.0, 100.0], [25.0, 25.0, -100.0]);
        assert!((front.fraction - 0.45).abs() < 1e-7, "{front:?}");
        assert_eq!(front.normal, [0.0, 0.0, 1.0]);
        assert_eq!(front.surface_flags, 0x0050_0100);
        assert_eq!(front.contents, 1);
        assert!(!front.start_solid && !front.all_solid);
        let back = c.raycast([25.0, 25.0, -100.0], [25.0, 25.0, 100.0]);
        assert!((back.fraction - 0.55).abs() < 1e-7, "{back:?}");
        assert_eq!(back.normal, [0.0, 0.0, -1.0]);
        let parallel = c.raycast([25.0, 25.0, 11.0], [35.0, 25.0, 11.0]);
        assert_eq!(parallel.contents, 0, "{parallel:?}");
        let beyond_face = c.raycast([60.0, 60.0, 20.0], [60.0, 60.0, 0.0]);
        assert_eq!(beyond_face.contents, 0, "{beyond_face:?}");
    }
    #[test]
    fn raycast_terrain_includes_edges_vertices_and_coplanar_entry() {
        let c = world(
            vec![],
            vec![TriangleExport {
                vertices: [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
                contents: 1,
                surface_flags: 7,
            }],
        );
        for point in [[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [100.0, 0.0]] {
            let hit = c.raycast([point[0], point[1], 10.0], [point[0], point[1], -10.0]);
            assert_eq!(hit.fraction, 0.5, "point={point:?}, {hit:?}");
            assert_eq!(hit.contents, 1);
        }
        let coplanar = c.raycast([-25.0, 25.0, 0.0], [75.0, 25.0, 0.0]);
        assert_eq!(coplanar.fraction, 0.25, "{coplanar:?}");
        assert_eq!(coplanar.contents, 1);
        let stationary = c.raycast([25.0, 25.0, 0.0], [25.0, 25.0, 0.0]);
        assert_eq!(stationary.fraction, 0.0);
        assert_eq!(stationary.contents, 1);
        assert!(!stationary.start_solid && !stationary.all_solid);
        let outside = c.raycast([60.0, 60.0, 0.0], [70.0, 70.0, 0.0]);
        assert_eq!(outside.contents, 0, "{outside:?}");
    }
    #[test]
    fn raycast_uses_shot_contents_while_player_sweeps_keep_their_own_mask() {
        // Exercise each shot-only category separately: otherwise the older
        // C1 mask's missing 0x8000 bit is hidden by an accompanying clipshot bit.
        for shot_contents in [0x0000_2000, 0x0000_8000] {
            let mut player_clip = brush([20.0, 0.0, 35.0], [5.0, 100.0, 35.0]);
            player_clip.contents = 0x0001_0000;
            let mut shot_clip = brush([100.0, 0.0, 35.0], [5.0, 100.0, 35.0]);
            shot_clip.contents = shot_contents;
            let c = world(vec![player_clip, shot_clip], vec![]);
            let hit = c.raycast([0.0, 0.0, 30.0], [200.0, 0.0, 30.0]);
            assert!((hit.fraction - 95.0 / 200.0).abs() < 1e-7, "{hit:?}");
            assert_eq!(hit.contents, shot_contents);
            let player = c.trace([50.0, 0.0, 0.0], [200.0, 0.0, 0.0], 70.0);
            assert_eq!(player.fraction, 1.0, "{player:?}");
            let player = c.trace([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], 70.0);
            assert!(player.fraction < 1.0, "{player:?}");
            assert_eq!(player.contents, 0x0001_0000);
        }
    }

    #[test]
    fn brush_material_comes_from_the_actual_entry_face() {
        let mut wall = brush([100.0, 0.0, 30.0], [10.0, 10.0, 10.0]);
        wall.plane_surface_flags = vec![1 << 20, 3 << 20, 11 << 20, 12 << 20, 18 << 20, 23 << 20];
        let c = world(vec![wall], vec![]);
        for (start, end, surface) in [
            ([0.0, 0.0, 30.0], [200.0, 0.0, 30.0], 3),
            ([200.0, 0.0, 30.0], [0.0, 0.0, 30.0], 1),
            ([100.0, 0.0, 100.0], [100.0, 0.0, 0.0], 18),
        ] {
            assert_eq!(c.raycast(start, end).surface_flags, surface << 20);
        }
        let sweep = c.trace([0.0, 0.0, 30.0], [200.0, 0.0, 30.0], 20.0);
        assert_eq!(sweep.surface_flags, 3 << 20);
        let mut slope = brush([0.0; 3], [100.0; 3]);
        slope.planes = vec![[0.70710677, 0.0, 0.70710677, 0.0]];
        slope.plane_surface_flags = vec![7 << 20, 0, 0, 0, 0, 0, 0];
        let c = world(vec![slope], vec![]);
        assert_eq!(
            c.raycast([0.0, 0.0, 80.0], [0.0, 0.0, -80.0]).surface_flags,
            7 << 20
        );
    }
}
