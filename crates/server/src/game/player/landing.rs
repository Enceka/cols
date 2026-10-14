//! Native0x417cf0/0x417e30 landing energy, movement penalty and event packing.
use codol_protocol::game::snapshot::events::Event;
use codol_protocol::game::snapshot::events::JUMP;
use codol_protocol::game::snapshot::events::LANDING_HARD;
use codol_protocol::game::snapshot::events::LANDING_PAIN;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Landing {
    pub time: u32,
    pub damage: u16,
    pub pm_time: u16,
    pub velocity_scale: f32,
    pub event: Option<Event>,
}

pub(crate) fn calculate(
    time: u32,
    previous_z: f32,
    previous_velocity_z: f32,
    origin_z: f32,
    gravity: u16,
    surface_flags: u32,
) -> Option<Landing> {
    if gravity == 0
        || ![previous_z, previous_velocity_z, origin_z]
            .into_iter()
            .all(f32::is_finite)
    {
        return None;
    }
    let g = f32::from(gravity);
    let a = -0.5 * g;
    let distance = previous_z - origin_z;
    let discriminant =
        (f64::from(previous_velocity_z).powi(2) - f64::from(a) * 4.0 * f64::from(distance)) as f32;
    if discriminant < 0.0 || !discriminant.is_finite() {
        return None;
    }
    let interval = ((-f64::from(previous_velocity_z) - f64::from(discriminant).sqrt())
        / (2.0 * f64::from(a))) as f32;
    let impact = (f64::from(interval) * -f64::from(g) + f64::from(previous_velocity_z)) as f32;
    let height = (f64::from(impact).powi(2) / (f64::from(g) * 2.0)) as f32;
    let maximum = if surface_flags & 0x1000 != 0 { 10 } else { 100 };
    let damage = if height > 128.0 && surface_flags & 1 == 0 {
        if height >= 300.0 {
            maximum
        } else {
            ((f64::from(height) - 128.0) / 172.0 * f64::from(maximum)) as u16
        }
    } else {
        0
    };
    let intensity = if height > 12.0 {
        (((f64::from(height) - 12.0) / 26.0) * 4.0 + 4.0).min(24.0) as u32
    } else {
        0
    };
    let surface = if surface_flags & 0x2000 == 0 {
        (surface_flags >> 20) & 31
    } else {
        0
    };
    let (pm_time, velocity_scale) = if damage > 0 && damage < 100 && surface_flags & 2 == 0 {
        let pm_time = (damage * 35 + 500).min(2000);
        let scale = if pm_time >= 1500 {
            0.2
        } else {
            (0.5 - (f64::from(pm_time) - 500.0) / 1000.0 * 0.3) as f32
        };
        (pm_time, scale)
    } else if damage > 0 || height >= 12.0 {
        (0, 0.67_f32)
    } else {
        (0, 1.0)
    };
    let event = if surface == 0 || height <= 4.0 {
        None
    } else {
        let (id, parameter) = if damage > 0 {
            (
                LANDING_PAIN,
                (u32::from(damage) | if surface_flags & 0x1000 != 0 { 128 } else { 0 }) << 5
                    | surface,
            )
        } else if height >= 12.0 {
            (LANDING_HARD, intensity << 5 | surface)
        } else {
            // Native0x41818d and0x4181bc both use EV_JUMP for a soft landing.
            (JUMP, surface)
        };
        Some(Event { id, parameter })
    };
    Some(Landing {
        time,
        damage,
        pm_time,
        velocity_scale,
        event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fall(height: f32, flags: u32) -> Landing {
        calculate(2000, height, 0.0, 0.0, 800, flags | 3 << 20).unwrap()
    }
    #[test]
    fn native_height_boundaries_and_surface_caps_keep_correct_event_widths() {
        assert_eq!(fall(128.0, 0).damage, 0);
        assert_eq!(fall(214.0, 0).damage, 50);
        assert_eq!(fall(300.0, 0).damage, 100);
        assert_eq!(fall(300.0, 0x1000).damage, 10);
        assert_eq!(fall(500.0, 1).damage, 0);
        assert_eq!(fall(214.0, 0).pm_time, 2000);
        assert_eq!(fall(214.0, 2).pm_time, 0);
        let soft = fall(10.0, 0).event.unwrap();
        assert_eq!((soft.id, soft.parameter), (JUMP, 3));
        assert!(fall(3.0, 0).event.is_none());
        let pain = fall(300.0, 0x1000).event.unwrap();
        assert_eq!(
            (pain.id, pain.parameter),
            (LANDING_PAIN, (10 | 128) << 5 | 3)
        );
        for height in [10.0, 50.0, 214.0, 500.0] {
            fall(height, 0).event.unwrap().validate().unwrap();
            assert!(fall(height, 0x2000).event.is_none());
        }
    }
    #[test]
    fn impact_energy_uses_incoming_velocity_and_partial_command_distance() {
        let expected = fall(210.0, 0).damage;
        let velocity = -560.0;
        assert_eq!(
            calculate(2000, 14.0, velocity, 0.0, 800, 3 << 20)
                .unwrap()
                .damage,
            expected
        );
        assert!(calculate(2000, 0.0, 0.0, 1.0, 800, 0).is_none());
        assert!(calculate(2000, 0.0, 0.0, 0.0, 0, 0).is_none());
    }
}
