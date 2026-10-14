//! Server-owned motion against the exported 3.3.3.3 map geometry.
//!
//! Target evidence: Pmove 0x41e860 (66 ms chops), PM_CmdScale 0x416aa0,
//! jump edge 0x40eac0, clipMap 0x02dfb000, brush contents 0x5888e0 and
//! terrain vertex blocks 0x5881f3. The upright capsule has radius 15 and a
//! vertical center segment of length `height - 30`. Brush planes use that
//! capsule's support function; terrain sweeps its full face/edge/vertex distance.
//! This does not claim exact parity with the client's mesh-border filtering.
//! Export coverage is world brushes and terrain. Registered destructible props
//! use explicit oriented bounds from their native models; other model meshes
//! and moving entities are outside this geometry export.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use super::math::*;
pub const PLAYER_MASK: u32 = 0x0281_4011;
/// Target 3.3.3.3 bullet trace mask (pushes at 0x55a4a0 / 0x55a4c3).
/// It includes clipshot geometry and excludes player-only clip volumes.
pub const SHOT_MASK: u32 = 0x0280_e831;
pub const WORLD: u16 = 2046;
pub const NONE: u16 = 2047;
pub(crate) const SKIN: f32 = 0.03125;
const CLIENT_SHA: &str = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b";

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
    /// Native cbrush+2 is one-based; zero denotes permanent geometry.
    #[serde(default)]
    glass_piece: Option<u16>,
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

#[derive(Clone, Debug)]
enum Shape {
    Brush {
        planes: Vec<[f32; 4]>,
        surfaces: Vec<u32>,
    },
    Triangle {
        vertices: [V3; 3],
    },
}
#[derive(Clone, Debug)]
struct Primitive {
    glass_piece: Option<u16>,
    bounds: Bounds,
    contents: u32,
    surface_flags: u32,
    shape: Shape,
}
#[derive(Clone, Debug)]
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
    primitives: Arc<Vec<Primitive>>,
    indices: Arc<Vec<usize>>,
    nodes: Arc<Vec<Node>>,
    known_glass: [u64; 16],
    destroyed_glass: [AtomicU64; 16],
    ignore_glass: bool,
    props: Arc<RwLock<Vec<(u16, Primitive)>>>,
}

impl Clone for Collision {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            ncs_crc: self.ncs_crc,
            captured_contents_mask: self.captured_contents_mask,
            primitives: self.primitives.clone(),
            indices: self.indices.clone(),
            nodes: self.nodes.clone(),
            known_glass: self.known_glass,
            destroyed_glass: std::array::from_fn(|i| {
                AtomicU64::new(self.destroyed_glass[i].load(Ordering::Relaxed))
            }),
            ignore_glass: self.ignore_glass,
            props: self.props.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Trace {
    pub prop_entity: Option<u16>,
    pub glass_piece: Option<u16>,
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
            prop_entity: None,
            glass_piece: None,
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
    /// Each local world owns destruction. The map template and other rooms
    /// share only immutable geometry; a new match begins with intact glass.
    pub(crate) fn fresh_session(&self) -> Self {
        Self {
            props: Arc::new(RwLock::new(Vec::new())),
            destroyed_glass: std::array::from_fn(|_| AtomicU64::new(0)),
            ignore_glass: false,
            ..self.clone()
        }
    }

    /// Planning can approach a breakable pane, but movement and damage always
    /// trace the real world. This view never destroys a gameplay object.
    pub(crate) fn navigation_view(&self) -> Self {
        Self {
            ignore_glass: true,
            ..self.clone()
        }
    }

    fn glass_is_absent(&self, piece: Option<u16>) -> bool {
        piece.is_some_and(|p| {
            self.ignore_glass
                || self.destroyed_glass[usize::from(p) / 64].load(Ordering::Relaxed)
                    & (1 << (p % 64))
                    != 0
        })
    }

    pub(crate) fn shatter_glass(&self, piece: u16) -> bool {
        if piece >= 1024 || self.ignore_glass {
            return false;
        }
        let index = usize::from(piece) / 64;
        let bit = 1 << (piece % 64);
        if self.known_glass[index] & bit == 0 {
            return false;
        }
        let changed = self.destroyed_glass[index].fetch_or(bit, Ordering::Relaxed) & bit == 0;
        if changed {
            tracing::info!(
                map = self.map,
                piece,
                "native glass shattered by authoritative hit"
            );
        }
        changed
    }

    pub(crate) fn destroyed_glass(&self) -> Vec<u16> {
        self.destroyed_glass
            .iter()
            .enumerate()
            .flat_map(|(i, word)| {
                let word = word.load(Ordering::Relaxed);
                (0..64)
                    .filter(move |bit| word & (1 << bit) != 0)
                    .map(move |bit| (i * 64 + bit) as u16)
            })
            .collect()
    }

    /// A validated bullet breaks only mapped glass before its closest body.
    /// Recast after each shatter; permanent WINDOW brushes still stop bullets.
    pub(crate) fn shot_raycast(&self, start: V3, end: V3, body_fraction: f32) -> Trace {
        self.shot_raycast_excluding_prop(start, end, body_fraction, None)
    }
    pub(crate) fn shot_raycast_excluding_prop(
        &self,
        start: V3,
        end: V3,
        body_fraction: f32,
        ignore: Option<u16>,
    ) -> Trace {
        loop {
            let hit = self.raycast_excluding_prop(start, end, ignore);
            if hit.start_solid || hit.all_solid || hit.fraction >= body_fraction {
                return hit;
            }
            let Some(piece) = hit.glass_piece else {
                return hit;
            };
            if !self.shatter_glass(piece) {
                return hit;
            }
        }
    }

    pub(crate) fn visibility_raycast(&self, start: V3, end: V3) -> Trace {
        self.navigation_view().raycast(start, end)
    }

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
        let mut known_glass = [0; 16];
        for b in export.brushes {
            if let Some(piece) = b.glass_piece {
                ensure!(
                    piece < 1024 && b.contents & 0x10 != 0,
                    "invalid native glass brush identity"
                );
                known_glass[usize::from(piece) / 64] |= 1 << (piece % 64);
            }
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
                    glass_piece: b.glass_piece,
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
                glass_piece: None,
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
            props: Arc::new(RwLock::new(Vec::new())),
            map: export.map,
            ncs_crc: export.ncs_crc,
            captured_contents_mask: export.captured_contents_mask,
            indices: Arc::new((0..primitives.len()).collect()),
            primitives: Arc::new(primitives),
            nodes: Arc::new(Vec::new()),
            known_glass,
            destroyed_glass: std::array::from_fn(|_| AtomicU64::new(0)),
            ignore_glass: false,
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
        Arc::make_mut(&mut self.nodes).push(Node::Leaf {
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
            Arc::make_mut(&mut self.indices)[first..first + count].sort_unstable_by(|&a, &b| {
                self.primitives[a]
                    .bounds
                    .center(axis)
                    .total_cmp(&self.primitives[b].bounds.center(axis))
            });
            let left = self.build(first, count / 2);
            let right = self.build(first + count / 2, count - count / 2);
            Arc::make_mut(&mut self.nodes)[node] = Node::Branch {
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
        self.trace_props(start, end, Some(capsule), PLAYER_MASK, None, &mut result);
        result
    }
    /// Cast a zero-width segment through exported world geometry. Unlike the
    /// player sweep, this applies neither the capsule radius nor movement skin.
    /// A point beginning inside a solid brush reports fraction zero even when
    /// its endpoint escapes, so a shot cannot originate through that brush.
    pub fn raycast(&self, start: V3, end: V3) -> Trace {
        self.raycast_mask(start, end, SHOT_MASK)
    }
    pub(crate) fn replace_props(&self, colliders: &[crate::game::world::props::Collider]) {
        self.replace_prop_range(256..384, colliders);
    }
    /// Map objects and reward devices own disjoint collision ranges. Updating
    /// one range must preserve every other owner's live shapes.
    pub(crate) fn replace_prop_range(
        &self,
        range: std::ops::Range<u16>,
        colliders: &[crate::game::world::props::Collider],
    ) {
        assert!(colliders.iter().all(|p| range.contains(&p.entity)));
        let props = colliders
            .iter()
            .map(|p| {
                (
                    p.entity,
                    Primitive {
                        glass_piece: None,
                        bounds: Bounds {
                            min: p.min,
                            max: p.max,
                        },
                        contents: 1,
                        surface_flags: 12 << 20,
                        shape: Shape::Brush {
                            planes: p.planes.clone(),
                            surfaces: vec![12 << 20; 6],
                        },
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut live = self.props.write().expect("prop geometry lock");
        live.retain(|(entity, _)| !range.contains(entity));
        live.extend(props);
        live.sort_by_key(|(entity, _)| *entity);
    }
    /// Conservative broad bounds for infrequent device placement. Brushes use
    /// their face planes; triangles use all box/triangle separating axes.
    pub(crate) fn bounds_clear(&self, min: V3, max: V3) -> bool {
        let bounds = Bounds { min, max };
        valid(min)
            && valid(max)
            && (0..3).all(|i| min[i] < max[i])
            && !self.bounds_blocked(0, bounds)
            && !self
                .props
                .read()
                .expect("prop geometry lock")
                .iter()
                .any(|(_, p)| shape_blocks_bounds(p, bounds))
    }
    fn bounds_blocked(&self, node: usize, bounds: Bounds) -> bool {
        if !self.nodes[node].bounds().intersects(bounds) {
            return false;
        }
        match self.nodes[node] {
            Node::Branch { left, right, .. } => {
                self.bounds_blocked(left, bounds) || self.bounds_blocked(right, bounds)
            }
            Node::Leaf { first, count, .. } => {
                self.indices[first..first + count].iter().any(|&index| {
                    let p = &self.primitives[index];
                    p.contents & PLAYER_MASK != 0
                        && !self.glass_is_absent(p.glass_piece)
                        && shape_blocks_bounds(p, bounds)
                })
            }
        }
    }
    pub(crate) fn raycast_excluding_prop(&self, start: V3, end: V3, ignore: Option<u16>) -> Trace {
        self.raycast_mask_excluding(start, end, SHOT_MASK, ignore)
    }
    fn trace_props(
        &self,
        start: V3,
        end: V3,
        capsule: Option<Capsule>,
        mask: u32,
        ignore: Option<u16>,
        result: &mut Trace,
    ) {
        let half = capsule.map_or([0.0; 3], |c| {
            [
                c.radius + SKIN,
                c.radius + SKIN,
                c.radius + c.segment_half + SKIN,
            ]
        });
        let bounds = Bounds::center_half(start, half).union(Bounds::center_half(end, half));
        for (id, p) in self.props.read().expect("prop geometry lock").iter() {
            if Some(*id) == ignore || p.contents & mask == 0 || !p.bounds.intersects(bounds) {
                continue;
            }
            let Shape::Brush { planes, surfaces } = &p.shape else {
                continue;
            };
            let hit = match capsule {
                Some(c) => {
                    let hit = sweep_brush(planes, surfaces, start, end, c);
                    (hit.fraction < 1.0 || hit.start_solid || hit.all_solid).then_some(hit)
                }
                None => ray_brush(planes, surfaces, start, end),
            };
            let Some(mut hit) = hit else {
                continue;
            };
            let was_solid = result.start_solid;
            result.start_solid |= hit.start_solid;
            result.all_solid |= hit.all_solid;
            if hit.fraction < result.fraction
                || hit.fraction == result.fraction
                    && (result.contents == 0 || hit.start_solid && !was_solid)
            {
                hit.contents = p.contents;
                hit.prop_entity = Some(*id);
                hit.start_solid |= result.start_solid;
                hit.all_solid |= result.all_solid;
                *result = hit;
            }
        }
    }
    /// Navigation support must use the same contents as the player's hull.
    /// Bullet/visibility-only volumes can overlap physically walkable terrain.
    pub(crate) fn ground_raycast(&self, start: V3, end: V3) -> Trace {
        self.raycast_mask(start, end, PLAYER_MASK)
    }
    pub(crate) fn projectile_trace(&self, start: V3, end: V3, radius: f32) -> Trace {
        self.projectile_trace_excluding_prop(start, end, radius, None)
    }
    pub(crate) fn projectile_trace_excluding_prop(
        &self,
        start: V3,
        end: V3,
        radius: f32,
        ignore: Option<u16>,
    ) -> Trace {
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
        self.trace_props(start, end, Some(capsule), SHOT_MASK, ignore, &mut trace);
        trace
    }
    pub fn movement_surface(&self, start: V3, end: V3) -> Trace {
        self.raycast_mask(start, end, PLAYER_MASK)
    }
    fn raycast_mask(&self, start: V3, end: V3, mask: u32) -> Trace {
        self.raycast_mask_excluding(start, end, mask, None)
    }
    fn raycast_mask_excluding(&self, start: V3, end: V3, mask: u32, ignore: Option<u16>) -> Trace {
        let bounds = Bounds {
            min: std::array::from_fn(|i| start[i].min(end[i])),
            max: std::array::from_fn(|i| start[i].max(end[i])),
        };
        let mut result = Trace::default();
        self.raycast_node(0, bounds, start, end, mask, &mut result);
        self.trace_props(start, end, None, mask, ignore, &mut result);
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
                    if primitive.contents & mask == 0
                        || self.glass_is_absent(primitive.glass_piece)
                        || !primitive.bounds.intersects(bounds)
                    {
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
                        result.glass_piece = primitive.glass_piece;
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
                    if p.contents & sweep.mask == 0
                        || self.glass_is_absent(p.glass_piece)
                        || !p.bounds.intersects(bounds)
                    {
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
                        result.glass_piece = p.glass_piece;
                    }
                }
            }
        }
    }
}

fn shape_blocks_bounds(p: &Primitive, bounds: Bounds) -> bool {
    if !p.bounds.intersects(bounds) {
        return false;
    }
    let center = std::array::from_fn(|i| bounds.center(i));
    let half: V3 = std::array::from_fn(|i| (bounds.max[i] - bounds.min[i]) * 0.5);
    match &p.shape {
        Shape::Brush { planes, .. } => !planes.iter().any(|plane| {
            let n = [plane[0], plane[1], plane[2]];
            dot(n, center) - (0..3).map(|i| n[i].abs() * half[i]).sum::<f32>() >= plane[3]
        }),
        Shape::Triangle { vertices } => {
            let points = vertices.map(|v| sub(v, center));
            let edges = [
                sub(points[1], points[0]),
                sub(points[2], points[1]),
                sub(points[0], points[2]),
            ];
            let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let separated = |axis: V3| {
                let projections = points.map(|v| dot(axis, v));
                let radius = (0..3).map(|i| axis[i].abs() * half[i]).sum::<f32>();
                projections.into_iter().fold(f32::INFINITY, f32::min) > radius
                    || projections.into_iter().fold(f32::NEG_INFINITY, f32::max) < -radius
            };
            !separated(cross(edges[0], edges[1]))
                && !axes.into_iter().any(separated)
                && !edges
                    .into_iter()
                    .any(|edge| axes.into_iter().any(|axis| separated(cross(edge, axis))))
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

#[cfg(test)]
mod tests;
