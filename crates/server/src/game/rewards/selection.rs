//! A selection reserves one inventory instance. No attack is created before
//! all required locations pass a real world trace and the ticket is committed.
use super::inventory::Ticket;
use super::killstreaks::RadarResources;
use crate::content::maps::minimap::Minimap;
use crate::game::world::collision::Collision;
use codol_protocol::game::channel::usercmd::{
    BUTTON_LOCATION_CANCEL, BUTTON_LOCATION_CONFIRM, UserCommand,
};
use codol_protocol::game::snapshot::location_selection::LocationSelection;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Point {
    pub origin: [f32; 3],
    pub yaw: f32,
    pub coverage: Option<f32>,
}
pub(crate) enum Action {
    None,
    Cancel,
    Complete,
}
pub(crate) struct Selection {
    pub ticket: Ticket,
    pub weapon: u16,
    pub life: u32,
    pub points: Vec<Point>,
    snapshot: LocationSelection,
    ready: u32,
    last_time: u32,
    buttons: u32,
    count: usize,
    trace_height: f32,
    coverage: Option<f32>,
}
impl Selection {
    pub fn new(
        ticket: Ticket,
        life: u32,
        input: &UserCommand,
        r: &RadarResources,
        map: &Minimap,
    ) -> Option<Self> {
        let id = ticket.native_id();
        let (name, directional, count) = match id {
            6 | 11 => ("map_artillery_selector", true, 1),
            7 => ("map_artillery_selector", false, 1),
            17 => ("map_mortar_selector", false, 3),
            _ => return None,
        };
        let radio = r.radio(id)?;
        //0x439287..0x4392c9 draws a square with diameter=radiusBits/31
        // times compass height. Keep its quantized world radius as the single
        // bound used by the bomb pattern and damage, including map scaling.
        let requested = super::strikes::coverage_radius(id);
        let bits = (2.0 * requested / map.world_height() * 31.0)
            .ceil()
            .clamp(1.0, 31.0) as u8;
        let snapshot = LocationSelection {
            shader: *r.selectors.get(name)?,
            directional,
            radius: bits,
        };
        snapshot.native_byte().ok()?;
        Some(Self {
            ticket,
            weapon: radio.weapon,
            life,
            points: Vec::new(),
            snapshot,
            ready: input
                .time
                .saturating_add(u32::from(radio.switch.raise_msec)),
            last_time: input.time,
            buttons: input.buttons,
            count,
            trace_height: r.air_route.as_ref()?.center[2],
            coverage: (id != 17).then_some(f32::from(bits) / 31.0 * map.world_height() * 0.5),
        })
    }
    pub fn snapshot(&self) -> LocationSelection {
        self.snapshot
    }
    pub fn command(&mut self, input: &UserCommand, map: &Minimap, collision: &Collision) -> Action {
        if input.time <= self.last_time {
            return Action::None;
        }
        self.last_time = input.time;
        let pressed = input.buttons & !self.buttons;
        self.buttons = input.buttons;
        if input.time < self.ready {
            return Action::None;
        }
        if pressed & BUTTON_LOCATION_CONFIRM != 0 {
            let (xy, mut yaw) = map.selected_position(input.selected);
            if !self.snapshot.directional {
                yaw = (input.time.wrapping_mul(1664525).wrapping_add(1013904223) % 360) as f32;
            }
            if let Some(origin) = ground(xy, self.trace_height, collision) {
                self.points.push(Point {
                    origin,
                    yaw,
                    coverage: self.coverage,
                });
                tracing::info!(time=input.time,id=self.ticket.native_id(),position=?origin,yaw,count=self.points.len(),required=self.count,"reward location accepted");
                if self.points.len() == self.count {
                    return Action::Complete;
                }
            }
        } else if pressed & BUTTON_LOCATION_CANCEL != 0 {
            return Action::Cancel;
        }
        Action::None
    }
}
pub(crate) fn ground(xy: [f32; 2], height: f32, c: &Collision) -> Option<[f32; 3]> {
    let start = [xy[0], xy[1], height];
    let end = [xy[0], xy[1], -32700.0];
    let trace = c.raycast(start, end);
    if trace.start_solid || trace.all_solid || trace.fraction >= 1.0 || trace.normal[2] < 0.35 {
        return None;
    }
    Some(std::array::from_fn(|i| {
        start[i] + (end[i] - start[i]) * trace.fraction
    }))
}
