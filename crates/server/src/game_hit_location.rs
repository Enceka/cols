//! Body-part ray classification from a captured current-client NPC pose.
//!
//! These are static pose boxes recovered from XModel+0x138 and DObj+0x98,
//! moved with the authoritative NPC origin/yaw. They do not evaluate skeletal
//! animation, stance changes, attachment models or native priority-map rules.
//! Weapon multipliers and world occlusion belong to the caller, not this file.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

const BODY_NAME: &str = "mp_fullbody_juggernaut_explosive_so";
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
type V3 = [f32; 3];

/// Current-client enum from0xcd83b8/0xcdf3c8, independently verified20 entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HitLocation {
    None,
    Helmet,
    Head,
    Neck,
    TorsoUpper,
    TorsoLower,
    RightArmUpper,
    LeftArmUpper,
    RightArmLower,
    LeftArmLower,
    RightHand,
    LeftHand,
    RightLegUpper,
    LeftLegUpper,
    RightLegLower,
    LeftLegLower,
    RightFoot,
    LeftFoot,
    Gun,
    Shield,
}

impl HitLocation {
    fn from_index(index: u8) -> Option<Self> {
        Some(match index {
            0 => Self::None,
            1 => Self::Helmet,
            2 => Self::Head,
            3 => Self::Neck,
            4 => Self::TorsoUpper,
            5 => Self::TorsoLower,
            6 => Self::RightArmUpper,
            7 => Self::LeftArmUpper,
            8 => Self::RightArmLower,
            9 => Self::LeftArmLower,
            10 => Self::RightHand,
            11 => Self::LeftHand,
            12 => Self::RightLegUpper,
            13 => Self::LeftLegUpper,
            14 => Self::RightLegLower,
            15 => Self::LeftLegLower,
            16 => Self::RightFoot,
            17 => Self::LeftFoot,
            18 => Self::Gun,
            19 => Self::Shield,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        [
            "none",
            "helmet",
            "head",
            "neck",
            "torso_upper",
            "torso_lower",
            "right_arm_upper",
            "left_arm_upper",
            "right_arm_lower",
            "left_arm_lower",
            "right_hand",
            "left_hand",
            "right_leg_upper",
            "left_leg_upper",
            "right_leg_lower",
            "left_leg_lower",
            "right_foot",
            "left_foot",
            "gun",
            "shield",
        ][self as usize]
    }

    pub fn is_head_region(self) -> bool {
        matches!(self, Self::Helmet | Self::Head)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionHit {
    /// Distance along the supplied finite start/end segment, in0..1.
    pub fraction: f32,
    pub point: V3,
    pub location: HitLocation,
    pub bone_index: u16,
}

#[derive(Deserialize)]
struct Capture {
    schema_version: u32,
    read_only: bool,
    client_sha256: String,
    connection_state: u32,
    ncs_crc: u32,
    entity_number: u16,
    entity_type: u8,
    model: u16,
    model_name: String,
    pose_kind: String,
    capture_origin: V3,
    capture_angles: V3,
    capture_view_offset: V3,
    bone_count: usize,
    root_bone_count: usize,
    bones: Vec<Bone>,
}

#[derive(Deserialize)]
struct Bone {
    index: u16,
    name: String,
    hit_location: u8,
    parent_index: Option<u16>,
    bounds_center: V3,
    bounds_half: V3,
    radius_squared: f32,
    pose_matrix: [f32; 8],
    pose_valid: bool,
    pose_translation_local: V3,
    pose_axes_local: [V3; 3],
}

#[derive(Clone, Debug)]
struct Region {
    index: u16,
    location: HitLocation,
    center: V3,
    axes: [V3; 3],
    half: V3,
}

#[derive(Clone, Debug)]
pub struct HitRegions {
    source_sha256: String,
    regions: Vec<Region>,
}

impl HitRegions {
    pub fn load(path: impl AsRef<Path>, expected_crc: u32) -> Result<Self> {
        let path = path.as_ref();
        let mut bytes = Vec::new();
        File::open(path)
            .with_context(|| format!("read hit-region capture {}", path.display()))?
            .take(MAX_CAPTURE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes, expected_crc)
            .with_context(|| format!("invalid hit-region capture {}", path.display()))
    }

    pub fn from_bytes(bytes: &[u8], expected_crc: u32) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_CAPTURE_BYTES,
            "hit-region capture size limit"
        );
        let capture: Capture =
            serde_json::from_slice(bytes).context("decode hit-region capture")?;
        ensure!(
            capture.schema_version == 1
                && capture.read_only
                && capture.client_sha256 == codol_protocol::CLIENT_SHA256
                && capture.connection_state == 10
                && capture.ncs_crc == expected_crc
                && capture.entity_number == 1
                && capture.entity_type == 1
                && capture.model == 109
                && capture.model_name == BODY_NAME
                && capture.pose_kind == "captured_static_pose",
            "hit-region capture identity mismatch"
        );
        ensure!(
            (1..=253).contains(&capture.bone_count)
                && capture.bones.len() == capture.bone_count
                && (1..=capture.bone_count).contains(&capture.root_bone_count)
                && finite(capture.capture_origin, 32768.0)
                && finite(capture.capture_view_offset, 1_000_000.0)
                && finite(capture.capture_angles, 3600.0),
            "invalid hit-region pose header"
        );
        ensure!(
            capture.bones[0].name == "tag_origin"
                && capture.bones.iter().any(|b| b.name == "j_head")
                && capture.bones.iter().any(|b| b.name == "j_spineupper"),
            "hit-region skeleton identity mismatch"
        );
        let yaw = capture.capture_angles[1];
        let mut resolved = Vec::with_capacity(capture.bone_count);
        let mut regions = Vec::new();
        for (index, bone) in capture.bones.iter().enumerate() {
            let location = HitLocation::from_index(bone.hit_location)
                .context("bone hit-location exceeds current enum")?;
            ensure!(
                usize::from(bone.index) == index
                    && !bone.name.is_empty()
                    && bone.name.len() <= 255
                    && bone
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && finite(bone.bounds_center, 4096.0)
                    && finite(bone.bounds_half, 4096.0)
                    && bone.bounds_half.iter().all(|h| *h >= 0.0)
                    && bone.radius_squared.is_finite()
                    && (0.0..=16_777_216.0).contains(&bone.radius_squared)
                    && bone
                        .pose_matrix
                        .iter()
                        .all(|n| n.is_finite() && n.abs() <= 1_000_000.0),
                "invalid hit-region bone metadata"
            );
            let parent = if index < capture.root_bone_count {
                ensure!(bone.parent_index.is_none(), "root bone has a parent");
                None
            } else {
                let parent = usize::from(bone.parent_index.context("non-root bone lacks parent")?);
                ensure!(parent < index, "bone parent must precede its child");
                Some(parent)
            };
            let location = if location == HitLocation::None {
                parent.map_or(HitLocation::None, |i| resolved[i])
            } else {
                location
            };
            resolved.push(location);
            let world_axes = matrix_axes(bone.pose_matrix);
            let valid = pose_valid(bone.pose_matrix, world_axes);
            ensure!(
                valid == bone.pose_valid,
                "inconsistent captured pose validity"
            );
            let axes = world_axes.map(|a| inverse_yaw(a, yaw));
            let translation = inverse_yaw(
                std::array::from_fn(|i| {
                    bone.pose_matrix[4 + i] + capture.capture_view_offset[i]
                        - capture.capture_origin[i]
                }),
                yaw,
            );
            ensure!(
                finite(bone.pose_translation_local, 1_000_000.0)
                    && (0..3)
                        .all(|i| (translation[i] - bone.pose_translation_local[i]).abs() <= 0.02)
                    && (0..3).all(|i| finite(bone.pose_axes_local[i], 2.0)
                        && (0..3)
                            .all(|j| (axes[i][j] - bone.pose_axes_local[i][j]).abs() <= 0.0001)),
                "captured local pose disagrees with current world matrix"
            );
            if !valid || bone.radius_squared == 0.0 || location == HitLocation::None {
                continue;
            }
            let center = std::array::from_fn(|i| {
                translation[i]
                    + (0..3)
                        .map(|j| axes[j][i] * bone.bounds_center[j])
                        .sum::<f32>()
            });
            ensure!(
                finite(center, 4096.0),
                "bone box outside local NPC region budget"
            );
            regions.push(Region {
                index: bone.index,
                location,
                center,
                axes,
                half: bone.bounds_half,
            });
        }
        let root = capture.bones[0].pose_matrix;
        let root_error = (0..3)
            .map(|i| {
                (f64::from(root[4 + i]) + f64::from(capture.capture_view_offset[i])
                    - f64::from(capture.capture_origin[i]))
                .powi(2)
            })
            .sum::<f64>();
        ensure!(
            root_error <= 4.0,
            "NPC moved between captured snapshot and rendered pose"
        );
        ensure!(
            regions.iter().any(|r| r.location.is_head_region())
                && regions.iter().any(|r| matches!(
                    r.location,
                    HitLocation::TorsoUpper | HitLocation::TorsoLower
                )),
            "capture lacks computed head or torso regions"
        );
        Ok(Self {
            source_sha256: format!("{:x}", Sha256::digest(bytes)),
            regions,
        })
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    /// Return the nearest captured body region on a finite world-space ray.
    /// Test the world trace's fraction separately before applying damage.
    pub fn classify(&self, start: V3, end: V3, origin: V3, yaw_degrees: f32) -> Option<RegionHit> {
        if !finite(start, 1_000_000.0)
            || !finite(end, 1_000_000.0)
            || !finite(origin, 32768.0)
            || !yaw_degrees.is_finite()
            || yaw_degrees.abs() > 3600.0
        {
            return None;
        }
        let local_start = inverse_yaw(sub(start, origin), yaw_degrees);
        let local_end = inverse_yaw(sub(end, origin), yaw_degrees);
        if dot(sub(local_end, local_start), sub(local_end, local_start)) < 1e-12 {
            return None;
        }
        let mut best: Option<RegionHit> = None;
        for region in &self.regions {
            let start = sub(local_start, region.center);
            let end = sub(local_end, region.center);
            let start = region.axes.map(|a| dot(start, a));
            let end = region.axes.map(|a| dot(end, a));
            if let Some(fraction) = ray_box(start, end, region.half)
                && best.as_ref().is_none_or(|hit| fraction < hit.fraction)
            {
                best = Some(RegionHit {
                    fraction,
                    point: [0.0; 3],
                    location: region.location,
                    bone_index: region.index,
                });
            }
        }
        best.map(|mut hit| {
            hit.point = std::array::from_fn(|i| start[i] + (end[i] - start[i]) * hit.fraction);
            hit
        })
    }
}

fn finite(v: V3, limit: f32) -> bool {
    v.iter().all(|n| n.is_finite() && n.abs() <= limit)
}
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: V3, b: V3) -> f32 {
    a.iter().zip(b).map(|(a, b)| *a * b).sum()
}
fn inverse_yaw(v: V3, yaw_degrees: f32) -> V3 {
    let (s, c) = f64::from(yaw_degrees).to_radians().sin_cos();
    [
        (c * f64::from(v[0]) + s * f64::from(v[1])) as f32,
        (-s * f64::from(v[0]) + c * f64::from(v[1])) as f32,
        v[2],
    ]
}
fn matrix_axes(matrix: [f32; 8]) -> [V3; 3] {
    let [x, y, z, w, _, _, _, weight] = matrix.map(f64::from);
    [
        [
            (1.0 - weight * (y * y + z * z)) as f32,
            (weight * (x * y + w * z)) as f32,
            (weight * (x * z - w * y)) as f32,
        ],
        [
            (weight * (x * y - w * z)) as f32,
            (1.0 - weight * (x * x + z * z)) as f32,
            (weight * (y * z + w * x)) as f32,
        ],
        [
            (weight * (x * z + w * y)) as f32,
            (weight * (y * z - w * x)) as f32,
            (1.0 - weight * (x * x + y * y)) as f32,
        ],
    ]
}
fn pose_valid(matrix: [f32; 8], axes: [V3; 3]) -> bool {
    let norm = matrix[..4]
        .iter()
        .map(|q| f64::from(*q).powi(2))
        .sum::<f64>();
    norm >= 1e-10
        && matrix[7] >= 0.0
        && (0..3).all(|i| {
            (i..3).all(|j| {
                (f64::from(dot(axes[i], axes[j])) - if i == j { 1.0 } else { 0.0 }).abs() <= 0.01
            })
        })
}
fn ray_box(start: V3, end: V3, half: V3) -> Option<f32> {
    let mut enter = 0.0f32;
    let mut leave = 1.0f32;
    for axis in 0..3 {
        let delta = end[axis] - start[axis];
        if delta.abs() < 1e-8 {
            if start[axis].abs() > half[axis] {
                return None;
            }
        } else {
            let a = (-half[axis] - start[axis]) / delta;
            let b = (half[axis] - start[axis]) / delta;
            enter = enter.max(a.min(b));
            leave = leave.min(a.max(b));
            if enter > leave {
                return None;
            }
        }
    }
    Some(enter)
}

/// Head box at z65..75, upper torso z18..62 and a left arm, for callers' tests.
#[cfg(test)]
pub(crate) fn synthetic() -> HitRegions {
    HitRegions::from_bytes(&serde_json::to_vec(&tests::capture()).unwrap(), 523897474).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub(super) fn capture() -> Value {
        let bone = |index, name, location, parent, point: V3, half: V3| {
            json!({
                "index":index,"name":name,"hit_location":location,"parent_index":parent,
                "bounds_center":[0,0,0],"bounds_half":half,"radius_squared":dot(half,half),
                "pose_matrix":[0,0,0,1,point[0]-1000.0,point[1]+500.0,point[2]-200.0,2],
                "pose_valid":true,"pose_translation_local":point,
                "pose_axes_local":[[1,0,0],[0,1,0],[0,0,1]]
            })
        };
        json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "connection_state":10,"ncs_crc":523897474,"entity_number":1,"entity_type":1,
            "model":109,"model_name":BODY_NAME,"pose_kind":"captured_static_pose",
            "capture_origin":[0,0,0],"capture_angles":[0,0,0],"capture_view_offset":[1000,-500,200],
            "bone_count":5,"root_bone_count":1,"bones":[
                bone(0,"tag_origin",0,None,[0.0;3],[0.0;3]),
                bone(1,"j_head",2,Some(0),[0.0,0.0,70.0],[5.0;3]),
                bone(2,"j_spineupper",4,Some(0),[0.0,0.0,40.0],[10.0,6.0,22.0]),
                bone(3,"j_shoulder_le",7,Some(2),[0.0,15.0,45.0],[4.0,5.0,12.0]),
                bone(4,"j_head_end",0,Some(1),[0.0,0.0,77.0],[3.0;3])
            ]
        })
    }
    fn load(value: &Value) -> Result<HitRegions> {
        HitRegions::from_bytes(&serde_json::to_vec(value).unwrap(), 523897474)
    }
    #[test]
    fn head_torso_arm_and_inherited_face_use_their_real_pose_boxes() {
        let regions = load(&capture()).unwrap();
        for (y, z, location, index) in [
            (0.0, 70.0, HitLocation::Head, 1),
            (0.0, 40.0, HitLocation::TorsoUpper, 2),
            (15.0, 45.0, HitLocation::LeftArmUpper, 3),
            (0.0, 79.0, HitLocation::Head, 4),
        ] {
            let hit = regions
                .classify([-100.0, y, z], [100.0, y, z], [0.0; 3], 0.0)
                .unwrap();
            assert_eq!((hit.location, hit.bone_index), (location, index));
            assert!((0.4..0.51).contains(&hit.fraction));
            assert!(hit.point[0] < 0.0);
        }
        assert!(
            regions
                .classify([-100.0, 50.0, 70.0], [100.0, 50.0, 70.0], [0.0; 3], 0.0)
                .is_none()
        );
    }
    #[test]
    fn nearest_body_surface_wins_even_when_the_ray_also_crosses_the_head() {
        let regions = load(&capture()).unwrap();
        let hit = regions
            .classify([0.0, 0.0, 0.0], [0.0, 0.0, 100.0], [0.0; 3], 0.0)
            .unwrap();
        assert_eq!(hit.location, HitLocation::TorsoUpper);
        assert!((hit.fraction - 0.18).abs() < 1e-6);
        let hit = regions
            .classify([0.0, 0.0, 100.0], [0.0, 0.0, 0.0], [0.0; 3], 0.0)
            .unwrap();
        assert_eq!(hit.location, HitLocation::Head);
        assert_eq!(hit.bone_index, 4);
    }
    #[test]
    fn rotated_bone_box_uses_its_axes_and_uncomputed_pose_boxes_do_not_hit() {
        let mut value = capture();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        value["bones"][1]["pose_matrix"] = json!([q, 0, 0, q, -1000, 500, -130, 2]);
        value["bones"][1]["pose_axes_local"] = json!([[1, 0, 0], [0, 0, 1], [0, -1, 0]]);
        value["bones"][1]["bounds_half"] = json!([5, 3, 5]);
        let regions = load(&value).unwrap();
        assert_eq!(
            regions
                .classify([-100.0, 4.0, 70.0], [100.0, 4.0, 70.0], [0.0; 3], 0.0)
                .unwrap()
                .location,
            HitLocation::Head
        );
        assert!(
            regions
                .classify([-100.0, 4.0, 75.0], [100.0, 4.0, 75.0], [0.0; 3], 0.0)
                .is_none()
        );
        value["bones"][1]["pose_matrix"] = json!([0, 0, 0, 0, 0, 0, 0, 0]);
        value["bones"][1]["pose_valid"] = json!(false);
        value["bones"][1]["pose_translation_local"] = json!([1000, -500, 200]);
        value["bones"][1]["pose_axes_local"] = json!([[1, 0, 0], [0, 1, 0], [0, 0, 1]]);
        let regions = load(&value).unwrap();
        assert!(
            regions
                .classify([-100.0, 4.0, 70.0], [100.0, 4.0, 70.0], [0.0; 3], 0.0)
                .is_none()
        );
    }
    #[test]
    fn source_world_offset_and_capture_yaw_are_removed_before_new_npc_placement() {
        let mut value = capture();
        // Rotate the captured current world matrices90 degrees. The local
        // regions stay identical; no inverse-bind-pose matrix participates.
        value["capture_origin"] = json!([2000, 3000, -100]);
        value["capture_angles"] = json!([0, 90, 0]);
        for bone in value["bones"].as_array_mut().unwrap() {
            let point: V3 = serde_json::from_value(bone["pose_translation_local"].clone()).unwrap();
            let q = std::f32::consts::FRAC_1_SQRT_2;
            bone["pose_matrix"] = json!([
                0,
                0,
                q,
                q,
                1000.0 - point[1],
                3500.0 + point[0],
                point[2] - 300.0,
                2
            ]);
        }
        let regions = load(&value).unwrap();
        let origin = [-300.0, 700.0, 50.0];
        let hit = regions
            .classify([-315.0, 600.0, 95.0], [-315.0, 800.0, 95.0], origin, 90.0)
            .unwrap();
        assert_eq!(hit.location, HitLocation::LeftArmUpper);
        assert!((hit.point[0] + 315.0).abs() < 0.001);
        assert_eq!(regions.source_sha256().len(), 64);
    }
    #[test]
    fn malformed_pose_identity_and_parent_data_are_rejected() {
        let original = capture();
        for (path, bad) in [
            ("/read_only", json!(false)),
            ("/ncs_crc", json!(1)),
            ("/model", json!(110)),
            ("/pose_kind", json!("inverse_bind_pose")),
            ("/bones/1/hit_location", json!(20)),
            ("/bones/1/parent_index", json!(1)),
            ("/bones/1/bounds_half/0", json!(-1)),
            ("/bones/1/pose_translation_local/2", json!(80)),
            ("/bones/1/pose_valid", json!(false)),
            ("/bones/1/pose_axes_local/0/0", json!(0)),
            ("/capture_origin/0", json!(100)),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(path).unwrap() = bad;
            assert!(load(&value).is_err(), "accepted {path}");
        }
        let regions = load(&original).unwrap();
        assert!(
            regions
                .classify([f32::NAN; 3], [0.0; 3], [0.0; 3], 0.0)
                .is_none()
        );
        assert!(
            regions
                .classify([0.0; 3], [0.0; 3], [0.0; 3], 0.0)
                .is_none()
        );
        assert!(
            regions
                .classify([0.0; 3], [1.0; 3], [0.0; 3], f32::NAN)
                .is_none()
        );
    }
}
