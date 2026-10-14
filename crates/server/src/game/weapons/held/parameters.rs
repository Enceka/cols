#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReloadTiming {
    pub normal_msec: u16,
    pub empty_msec: u16,
    pub add_msec: u16,
    pub empty_add_msec: u16,
    /// WeaponDef+0xd86 shell-by-shell reload; the normal timings are then
    /// one shell (anim13, +0x71c, ammo at +0x730).
    #[serde(default)]
    pub segmented: Option<SegmentedReload>,
}
impl ReloadTiming {
    pub fn valid(&self) -> bool {
        (1..=15000).contains(&self.normal_msec)
            && (1..=15000).contains(&self.empty_msec)
            && (1..=self.normal_msec).contains(&self.add_msec)
            && (1..=self.empty_msec).contains(&self.empty_add_msec)
            && self.segmented.is_none_or(|s| {
                s.start_msec <= 15000 && s.start_add_msec <= s.start_msec && s.end_msec <= 15000
            })
    }
}

/// Target 0x423490/0x424260/0x4243f0: start (state10, anim17, +0x740, one
/// shell at +0x744), shells (state8), end (state12, anim18, +0x748). Fire
/// interrupts as state11/9. Zero times skip that phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SegmentedReload {
    pub start_msec: u16,
    pub start_add_msec: u16,
    pub end_msec: u16,
}

/// Weapon states 8..=12 are reloading, its interrupt, start, start
/// interrupt and end (target 0x424660 groups them).
pub(super) fn reloading(state: u8) -> bool {
    (8..=12).contains(&state)
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdsTiming {
    pub in_rate: f32,
    pub out_rate: f32,
    pub reload_allowed: bool,
    pub reload_interruptible: bool,
    pub reload_end_msec: u16,
}
impl AdsTiming {
    pub fn valid(&self) -> bool {
        [self.in_rate, self.out_rate]
            .into_iter()
            .all(|r| r.is_finite() && r > 0.0 && r <= 1.0)
            && self.reload_end_msec <= 15000
    }
}

/// Target 0x4236a0 / 0x423b10 consume these timings from each weapon's
/// definition (+0x74c/+0x750/+0x768/+0x764), in milliseconds.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SwitchTiming {
    pub drop_msec: u16,
    pub raise_msec: u16,
    pub empty_drop_msec: u16,
    pub empty_raise_msec: u16,
}
impl SwitchTiming {
    pub fn valid(&self) -> bool {
        [
            self.drop_msec,
            self.raise_msec,
            self.empty_drop_msec,
            self.empty_raise_msec,
        ]
        .into_iter()
        .all(|time| (1..=15000).contains(&time))
    }
}

/// Native0x4283c0/0x428490; loop0x428440 has no duration.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SprintTiming {
    pub in_msec: u16,
    pub out_msec: u16,
}
impl SprintTiming {
    pub fn valid(self) -> bool {
        self.in_msec <= 15000 && self.out_msec <= 15000
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlternateTiming {
    pub drop_msec: u16,
    pub raise_msec: u16,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RechamberTiming {
    pub duration_msec: u16,
    pub eject_msec: u16,
}

/// Ordinary held-gun melee: WeaponDef+0x714/+0x6f4/+0x75c/+0x6e0.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MeleeTiming {
    pub duration_msec: u16,
    pub hit_msec: u16,
    pub recover_msec: u16,
    pub damage: u16,
    #[serde(default)]
    pub charge: Option<MeleeChargeTiming>,
}
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MeleeChargeTiming {
    pub duration_msec: u16,
    pub hit_msec: u16,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct MeleeApproach {
    pub yaw: u16,
    pub distance: u8,
}
impl MeleeTiming {
    pub fn valid(self) -> bool {
        (1..=15000).contains(&self.duration_msec)
            && self.hit_msec <= self.duration_msec
            && self.recover_msec <= 15000
            && self.damage > 0
            && self.charge.is_none_or(|c| {
                (1..=15000).contains(&c.duration_msec)
                    && (1..=c.duration_msec).contains(&c.hit_msec)
            })
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpreadTiming {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    pub decay: f32,
    pub fire_add: f32,
    pub turn_add: f32,
    pub move_add: f32,
    pub stance_decay: [f32; 3],
    pub ads: f32,
}
impl SpreadTiming {
    pub fn valid(self) -> bool {
        self.minimum
            .into_iter()
            .chain(self.maximum)
            .chain(self.stance_decay)
            .chain([
                self.decay,
                self.fire_add,
                self.turn_add,
                self.move_add,
                self.ads,
            ])
            .all(|v| v.is_finite() && (0.0..=180.0).contains(&v))
            && self
                .minimum
                .into_iter()
                .zip(self.maximum)
                .all(|(min, max)| min <= max)
    }
}

/// 0x429d20/0x429e50/0x429f20: contribution and shared-group ceiling.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub(crate) struct AmmoLimits {
    pub contribution: u32,
    pub shared: Option<[u32; 2]>,
    pub clip_only: bool,
    pub scavenger_allowed: bool,
}
