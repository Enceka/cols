use super::*;
use crate::game::rewards::{
    killstreaks::RadarResources,
    selection::{Action, Selection},
};
use codol_protocol::game::channel::usercmd::UserCommand;

impl TdmMatch {
    pub(crate) fn cancel_strike_selection(&mut self) {
        if let Some(selection) = self.selection.take() {
            self.streaks.cancel_radio(0, selection.ticket);
            tracing::info!(
                id = selection.ticket.native_id(),
                "reward selection cancelled; stock retained"
            );
        }
    }
    pub(crate) fn validate_strike_selection(&mut self, now: u32, alive: bool, weapon: Option<u16>) {
        if self.selection.as_ref().is_some_and(|s| {
            !alive
                || self.rules.finished
                || self.streaks.life(0) != s.life
                || weapon != Some(s.weapon)
                || !self
                    .streaks
                    .can_radio(0, self.config.teams[0], s.ticket.native_id(), now)
        }) {
            self.cancel_strike_selection();
        }
    }
    pub(crate) fn selection_snapshot(
        &self,
    ) -> Option<codol_protocol::game::snapshot::location_selection::LocationSelection> {
        self.selection.as_ref().map(Selection::snapshot)
    }
    /// None keeps the tablet open; Some(true) means one exact stock instance
    /// was committed; Some(false) means cancellation/rejection keeps the stock.
    pub(crate) fn select_strike(
        &mut self,
        id: u8,
        input: &UserCommand,
        r: &RadarResources,
        map: &crate::content::maps::minimap::Minimap,
        c: &Collision,
    ) -> Option<bool> {
        self.validate_strike_selection(input.time, true, Some(input.weapons[0]));
        if self.rules.finished {
            return Some(false);
        }
        if self.selection.is_none() {
            let ticket = self
                .streaks
                .reserve_radio(0, self.config.teams[0], id, input.time)?;
            let Some(s) = Selection::new(ticket, self.streaks.life(0), input, r, map) else {
                self.streaks.cancel_radio(0, ticket);
                return Some(false);
            };
            tracing::info!(id, weapon = s.weapon, "reward location selection opened");
            self.selection = Some(s);
            return None;
        }
        match self.selection.as_mut().unwrap().command(input, map, c) {
            Action::None => None,
            Action::Cancel => {
                self.cancel_strike_selection();
                Some(false)
            }
            Action::Complete => {
                let s = self.selection.take().unwrap();
                self.aircraft.reserve_slots(self.remote.vehicle_slots());
                let harrier = if id == 7 {
                    self.aircraft.plan_harrier(r, s.points[0], c)
                } else {
                    None
                };
                let plan = self.strikes.plan(
                    input.time,
                    id,
                    crate::game::rewards::strikes::Source {
                        owner: 0,
                        team: self.config.teams[0],
                        life: s.life,
                    },
                    &s.points,
                    r,
                    c,
                );
                if let Some(plan) = plan
                    && (id != 7 || harrier.is_some())
                    && self
                        .streaks
                        .commit_radio(0, self.config.teams[0], s.ticket, input.time)
                {
                    self.strikes.launch(plan);
                    if let Some(plan) = harrier {
                        self.aircraft.launch(
                            input.time,
                            plan,
                            crate::game::rewards::aircraft::Source {
                                owner: 0,
                                team: self.config.teams[0],
                                life: s.life,
                                serial: input.time,
                            },
                            c,
                        );
                    }
                    Some(true)
                } else {
                    self.streaks.cancel_radio(0, s.ticket);
                    Some(false)
                }
            }
        }
    }
    pub(super) fn advance_strikes(
        &mut self,
        now: u32,
        local: EnemySnapshot,
        height: f32,
        health: u16,
        emp: u8,
        c: &Collision,
    ) -> Tick {
        let mut tick = Tick::default();
        if self.rules.finished {
            self.dogs.clear(c);
        }
        let mut local_health = health;
        self.remote.advance(now, self.rules.finished, emp, c);
        let mut impacts = self.strikes.advance(now, self.rules.finished, emp, c);
        impacts.extend(self.remote.drain_hits());
        let targets = self
            .bots
            .iter()
            .filter(|b| b.health > 0)
            .map(|b| crate::game::rewards::drones::Target {
                actor: b.snapshot.client_index,
                team: b.snapshot.team,
                center: [
                    b.snapshot.origin[0],
                    b.snapshot.origin[1],
                    b.snapshot.origin[2] + 35.0,
                ],
                hidden: b.weapon.as_ref().is_some_and(|w| w.airplane_undetectable()),
                born: b.birth.max(self.rules.start.unwrap_or(b.birth)),
            })
            .collect::<Vec<_>>();
        impacts.extend(
            self.drones
                .advance(now, self.rules.finished, emp, &targets, c),
        );
        for hit in impacts {
            if self.rules.finished {
                break;
            }
            self.props.radial(now, hit.blast, c);
            self.deployables.radial(now, hit.blast, hit.team, c);
            self.aircraft.radial(now, hit.blast, hit.team, c);
            self.dogs.radial(now, hit.blast, hit.team, c);
            self.remote.radial_cars(hit.blast, hit.team, c);
            for source in self.airspace.radial(now, hit.blast, hit.team, c) {
                self.streaks.stop_flight(source);
            }
            let damage = self
                .bots
                .iter()
                .filter(|b| {
                    b.health > 0
                        && (b.snapshot.team != hit.team
                            || b.snapshot.client_index == hit.blast.owner)
                })
                .map(|b| {
                    (
                        b.snapshot.client_index,
                        crate::game::rewards::strikes::damage(hit, b.snapshot.origin, 40.0, c),
                        false,
                    )
                })
                .filter(|d| d.1 > 0)
                .collect::<Vec<_>>();
            self.damage_bots(
                now,
                CreditSource::reward(hit.blast.owner, hit.life),
                hit.blast.weapon,
                &damage,
                false,
            );
            if local.alive && local_health > 0 && (local.team != hit.team || hit.blast.owner == 0) {
                let d = crate::game::rewards::strikes::damage(hit, local.origin, height * 0.5, c);
                if d > 0 {
                    local_health = local_health.saturating_sub(d);
                    tick.hits.push(PlayerHit {
                        reward_life: Some(hit.life),
                        damage: d,
                        attacker: hit.blast.owner,
                        weapon: hit.blast.weapon,
                        headshot: false,
                    });
                }
            }
            tracing::debug!(owner=hit.blast.owner,life=hit.life,weapon=hit.blast.weapon,origin=?hit.blast.origin,"strike damage resolved");
        }
        tick
    }
}
