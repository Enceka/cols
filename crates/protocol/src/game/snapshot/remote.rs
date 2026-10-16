//! Target camera0x522b30 and input producer0x460070/0x45fe30.
use crate::lobby::channel::LobbyError;

pub const TAG_CONFIGSTRING_BASE: u16 = 5379;
pub const CONTROLS_FLAG: u32 = 0x80000;
pub const CAMERA_FLAG: u32 = 2;
pub const TAG_ANGLES_FLAG: u32 = 0x80;
pub const FOF_OVERLAY_FLAG: u32 = 0x10;
/// Target0x523893, consumed by the renderer's cg+0x16bfd0 thermal state.
pub const THERMAL_FLAG: u32 = 0x8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Camera {
    pub entity: u16,
    pub tag: u8,
    /// Copy tag orientation; false lets the native linked view turn locally.
    pub tag_angles: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Links {
    pub camera: Option<Camera>,
    pub controls: Option<u16>,
    pub fof_overlay: bool,
    pub thermal_vision: bool,
    /// Weapon-view attachment: PS.eFlags4 and viewlocked entity. Unlike the
    /// camera2 mode, target0x55cc78 permits native weapon cycling here.
    pub weapon_view: bool,
}
impl Links {
    pub fn validate(self) -> Result<(), LobbyError> {
        if self.camera.is_none() && self.controls.is_none()
            || self.fof_overlay && self.camera.is_none()
            || self.thermal_vision && self.camera.is_none()
            || self.weapon_view && (self.camera.is_none() || self.controls.is_some())
            || self.camera.is_some_and(|c| c.entity >= 2047 || c.tag >= 64)
            || self.controls.is_some_and(|entity| entity >= 2047)
        {
            return Err(LobbyError("invalid remote camera or controls link"));
        }
        Ok(())
    }
    pub(crate) fn camera_flags(self) -> u32 {
        self.camera.map_or(0, |c| {
            (if self.weapon_view { 0 } else { CAMERA_FLAG })
                | if self.thermal_vision { THERMAL_FLAG } else { 0 }
                | if c.tag_angles && !self.weapon_view {
                    TAG_ANGLES_FLAG
                } else {
                    0
                }
                | if self.fof_overlay {
                    FOF_OVERLAY_FLAG
                } else {
                    0
                }
        })
    }
    pub(crate) fn control_flags(self) -> u32 {
        if self.controls.is_some() {
            CONTROLS_FLAG
        } else {
            0
        }
    }
}
pub(crate) fn fields(links: Option<Links>) -> [(u32, u32, usize); 3] {
    [
        (
            168,
            links
                .filter(|l| !l.weapon_view)
                .and_then(|l| l.camera)
                .map_or(2047, |c| u32::from(c.entity)),
            11,
        ),
        (
            169,
            links
                .filter(|l| !l.weapon_view)
                .and_then(|l| l.camera)
                .map_or(0, |c| u32::from(c.tag)),
            6,
        ),
        (
            170,
            links.and_then(|l| l.controls).map_or(2047, u32::from),
            11,
        ),
    ]
}
