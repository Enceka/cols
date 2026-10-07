//! Minimal single-M4 ammo ownership. No reload, hit events, ADS or physics yet.
use codol_protocol::{
    game_snapshot::{Equipment, Firing},
    game_usercmd::UserCommand,
};

#[derive(Debug)]
pub(crate) struct Player {
    equipment: Equipment,
    fire_interval: Option<u16>,
    last_time: u32,
}

impl Player {
    pub fn new(mut equipment: Equipment, fire_interval: Option<u16>) -> Self {
        if fire_interval.is_some() {
            equipment.firing = Some(Firing {
                animation: 0x400,
                remaining_msec: 0,
                state: 0,
            });
        }
        Self {
            equipment,
            fire_interval,
            last_time: 1000,
        }
    }

    pub fn equipment(&self) -> Equipment {
        self.equipment
    }

    pub fn apply(&mut self, cmd: &UserCommand) {
        if cmd.time <= self.last_time {
            return;
        }
        let elapsed = cmd.time - self.last_time;
        self.last_time = cmd.time;
        let (Some(interval), Some(ammo), Some(firing)) = (
            self.fire_interval,
            self.equipment.ammo.as_mut(),
            self.equipment.firing.as_mut(),
        ) else {
            return;
        };
        firing.remaining_msec = firing
            .remaining_msec
            .saturating_sub(elapsed.min(u32::from(u16::MAX)) as u16);
        // Target 0x45f660: attack=1, menu capture=0x10000000.
        // Only the equipped M4 is supported, not offhand/melee/weapon switching.
        if firing.remaining_msec == 0 {
            if cmd.buttons & (1 | 4 | 0x10000000) == 1
                && cmd.weapons[0] == self.equipment.weapon
                && ammo.clip > 0
            {
                ammo.clip -= 1;
                // 0x4253e0 sets weaponTime from def+0xd10, state6. The zero
                // fire-delay M4 path is the only admitted timing profile.
                firing.remaining_msec = interval;
                firing.state = 6;
                firing.animation =
                    (firing.animation ^ 0x400) & 0x400 | if ammo.clip == 0 { 3 } else { 2 };
            } else if firing.state != 0 {
                firing.state = 0;
                firing.animation = (firing.animation ^ 0x400) & 0x400 | 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codol_protocol::game_snapshot::Ammo;
    fn player() -> Player {
        Player::new(
            Equipment {
                weapon: 352,
                viewmodel: None,
                firing: None,
                ammo: Some(Ammo {
                    clip_key: 352,
                    reserve_key: 163,
                    clip: 30,
                    reserve: 90,
                }),
            },
            Some(79),
        )
    }
    fn cmd(time: u32, buttons: u32) -> UserCommand {
        UserCommand {
            time,
            buttons,
            weapons: [352, 0, 0],
            ..Default::default()
        }
    }
    #[test]
    fn shots_obey_cooldown_and_duplicate_inputs_never_consume_twice() {
        let mut p = player();
        for c in [
            cmd(1010, 1),
            cmd(1010, 1),
            cmd(1005, 1),
            cmd(1050, 1),
            cmd(1088, 1),
        ] {
            p.apply(&c);
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&cmd(1089, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        p.apply(&cmd(1180, 0));
        for time in (1200..2200).step_by(50) {
            p.apply(&cmd(time, 0));
            assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        }
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 90);
    }
    #[test]
    fn empty_clip_menu_and_weapon_switch_do_not_create_ammo_or_shots() {
        let mut p = player();
        p.apply(&cmd(1100, 0x10000001));
        let mut other = cmd(1200, 1);
        other.weapons[0] = 180;
        p.apply(&other);
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
        for time in (1300..=6000).step_by(100) {
            p.apply(&cmd(time, 1));
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 0);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 90);
        assert_eq!(player().equipment().ammo.unwrap().clip, 30); // new match only
    }
}
