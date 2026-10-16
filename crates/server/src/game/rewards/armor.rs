//! Life-bound armored loadouts. Registered models, guns and weapon timings are
//! target captures.1000 health and0.65 movement are local compatibility rules;
//! normal availability remains gated on native acceptance and the real entry.
use super::{killstreaks::RadarResources, strikes::Source};
use crate::game::{
    combat::{Ballistics, DamageFalloff},
    weapons::held::{AdsTiming, HeldWeapons, ReloadTiming},
};
use codol_protocol::game::snapshot::{Ammo, Equipment, PlayerVitals, enemy::EnemySnapshot};

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperatorFacts {
    pub fire_type: u32,
    pub move_scale: f32,
    pub ads_move_scale: f32,
    pub reload: ReloadTiming,
    pub ads: Option<AdsTiming>,
    pub location_multipliers: [f32; 20],
}
impl OperatorFacts {
    pub(crate) fn valid(&self) -> bool {
        self.fire_type <= 6
            && self.reload.valid()
            && self.ads.is_none_or(|a| a.valid())
            && [self.move_scale, self.ads_move_scale]
                .into_iter()
                .all(|s| s.is_finite() && s > 0.0 && s <= 2.0)
            && self
                .location_multipliers
                .iter()
                .all(|s| s.is_finite() && (0.0..=100.0).contains(s))
    }
}
pub(crate) struct Plan {
    pub id: u8,
    model: u16,
    weapon: HeldWeapons,
}
impl Plan {
    pub(crate) fn new(r: &RadarResources, id: u8, team: u8, base: Equipment) -> Option<Self> {
        let (model, hands, name) = match (id, team) {
            (23, 1 | 2) => (
                "mp_fullbody_juggernaut_explosive_so",
                "viewhands_juggernaut_ally",
                "mg4juggernaut_mp",
            ),
            (24, 1) => (
                "c_kp_juggernaut_tf_00",
                "viewhands_player_kp_juggernaut_tf",
                "minigungunship_mp",
            ),
            (24, 2) => (
                "c_kp_juggernaut_sc_00",
                "viewhands_player_kp_juggernaut_sc",
                "minigungunship_mp",
            ),
            _ => return None,
        };
        let w = r.weapons.get(name)?;
        let facts = w.operator.as_ref()?;
        if !facts.valid()
            || w.weapon_type != 0
            || w.clip_size == 0
            || w.clip_size > 511
            || w.fire_interval_msec == 0
        {
            return None;
        }
        let falloff = DamageFalloff::from_stages(w.damage_stages, w.damage_ranges)?;
        let ballistics = Ballistics {
            damage: falloff.damage(0.0),
            range: 8192.0,
            near_range: w.damage_ranges[0],
            falloff: Some(falloff),
            location_multipliers: Some(facts.location_multipliers),
            pellets: 1,
            spread_degrees: 0.0,
        };
        if !ballistics.valid() {
            return None;
        }
        let model = *r.models.get(model)?;
        let hands = *r.models.get(hands)?;
        if !(1..1024).contains(&model) || !(1..1024).contains(&hands) {
            return None;
        }
        let mut e = base;
        e.weapon = w.weapon;
        e.viewmodel = Some(hands);
        e.inventory_weapon = None;
        e.inventory_override = None;
        e.alternate_weapon = None;
        e.alternates = [None; 2];
        e.secondary = None;
        e.offhands = [None; 2];
        e.offhand_weapon = 0;
        e.grenade_time = 0;
        e.events = None;
        e.firing = None;
        e.aim = None;
        e.fire_count = 0;
        e.queued_burst = false;
        e.barrel_flags = 0;
        e.needs_rechamber = false;
        e.spread_scale = 0.0;
        e.ammo = Some(Ammo {
            clip_key: w.clip_key?,
            reserve_key: w.ammo_key?,
            clip: w.clip_size,
            reserve: w.clip_size.saturating_mul(3).min(2047),
        });
        let mut weapon =
            HeldWeapons::new(e, Some(w.fire_interval_msec), Some(facts.reload), facts.ads);
        weapon.set_switch_timing(w.switch);
        weapon.set_fire_type(facts.fire_type).ok()?;
        weapon.set_move_scale(facts.move_scale);
        weapon.set_ads_move_scale(facts.ads_move_scale);
        weapon.set_move_modifier(0.65);
        weapon.set_ballistics(ballistics);
        weapon.set_reward_loadout();
        Some(Self { id, model, weapon })
    }
}
struct Active {
    source: Source,
    model: u16,
    saved: HeldWeapons,
    health: u16,
    maximum: u16,
}
#[derive(Default)]
pub(crate) struct Armor {
    active: Option<Active>,
    dead_model: Option<(u32, u16)>,
}
impl Armor {
    pub(crate) fn active(&self) -> bool {
        self.active.is_some()
    }
    pub(crate) fn source(&self) -> Option<Source> {
        self.active.as_ref().map(|a| a.source)
    }
    pub(crate) fn start(
        &mut self,
        mut plan: Plan,
        source: Source,
        now: u32,
        player: &mut HeldWeapons,
        vitals: &mut PlayerVitals,
    ) -> bool {
        if self.active() || vitals.health == 0 {
            return false;
        }
        plan.weapon.inherit_perks(player);
        let saved = player.clone();
        let active = Active {
            source,
            model: plan.model,
            saved,
            health: vitals.health,
            maximum: vitals.max_health,
        };
        tracing::info!(
            id = plan.id,
            owner = source.owner,
            life = source.life,
            "armored reward activated"
        );
        player.replace_loadout(plan.weapon, None);
        player.freeze_at(now);
        vitals.max_health = 1000;
        vitals.health = 1000;
        vitals.damage_count = 0;
        self.active = Some(active);
        true
    }
    pub(crate) fn appearance(&self, actor: &mut EnemySnapshot, life: u32) {
        let model = self
            .active
            .as_ref()
            .filter(|a| a.source.life == life)
            .map(|a| a.model)
            .or_else(|| {
                self.dead_model
                    .filter(|(old, _)| *old == life)
                    .map(|(_, m)| m)
            });
        if let Some(model) = model.filter(|_| self.active.is_some() || !actor.alive) {
            actor.model = model;
            actor.attachment_models = [0; 10];
            actor.attachment_tags = [0; 10];
        }
    }
    /// A dead or different life discards the saved equipment. A live match exit
    /// restores its exact ammo without healing damage taken inside the suit.
    pub(crate) fn validate(
        &mut self,
        now: u32,
        life: u32,
        finished: bool,
        player: &mut HeldWeapons,
        vitals: &mut PlayerVitals,
    ) -> bool {
        if self
            .dead_model
            .is_some_and(|(old, _)| old != life || vitals.health > 0)
        {
            self.dead_model = None;
        }
        let Some(a) = &self.active else {
            return false;
        };
        if a.source.life == life && vitals.health > 0 && !finished {
            return false;
        }
        let a = self.active.take().unwrap();
        if a.source.life != life || vitals.health == 0 {
            if vitals.health == 0
                && (a.source.life == life || a.source.life.checked_add(1) == Some(life))
            {
                self.dead_model = Some((life, a.model));
            }
            return false;
        }
        let health = (u32::from(vitals.health) * u32::from(a.maximum)
            / u32::from(vitals.max_health))
        .min(u32::from(a.health)) as u16;
        player.replace_loadout(a.saved, None);
        player.freeze_at(now);
        vitals.max_health = a.maximum;
        vitals.health = health.max(1);
        vitals.damage_count = 0;
        true
    }
}

#[cfg(test)]
mod tests;
