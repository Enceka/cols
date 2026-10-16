use super::*;
use crate::game::rewards::{killstreaks::RadarResources, strikes::Source};
use codol_protocol::game::channel::usercmd::UserCommand;

impl TdmMatch {
    pub(crate) fn call_drone(
        &mut self,
        now: u32,
        origin: [f32; 3],
        angles: [u16; 3],
        r: &RadarResources,
        c: &Collision,
    ) -> bool {
        if self.rules.finished
            || self.remote.busy()
            || self.selection.is_some()
            || !self.streaks.can_radio(0, self.config.teams[0], 26, now)
        {
            return false;
        }
        let Some(plan) = self.drones.plan(r, origin, angles, c) else {
            return false;
        };
        if !self.streaks.try_radio(0, self.config.teams[0], 26, now) {
            return false;
        }
        self.drones.launch(
            now,
            plan,
            Source {
                owner: 0,
                team: self.config.teams[0],
                life: self.streaks.life(0),
            },
        );
        true
    }
    pub(crate) fn validate_remote(&mut self, now: u32, alive: bool, weapon: Option<u16>) {
        let allowed =
            alive && !self.rules.finished && !self.streaks.emp_active(self.config.teams[0], now);
        if self
            .remote
            .pending()
            .is_some_and(|p| !allowed || p.life != self.streaks.life(0) || weapon != Some(p.weapon))
            && let Some(ticket) = self.remote.cancel_pending()
        {
            self.streaks.cancel_radio(0, ticket);
        }
        if self
            .remote
            .operator_life()
            .is_some_and(|life| !allowed || life != self.streaks.life(0))
        {
            self.remote.finish_operator();
        }
    }
    /// None means the terminal intro or control continues; false rejects use
    /// with the exact stock retained. Successful payload creation owns exit.
    pub(crate) fn remote_action(
        &mut self,
        now: u32,
        id: u8,
        input: &UserCommand,
        origin: [f32; 3],
        r: &RadarResources,
        c: &Collision,
    ) -> Option<bool> {
        self.validate_remote(now, true, Some(input.weapons[0]));
        if !matches!(id, 5 | 12 | 13 | 18) || self.rules.finished || self.selection.is_some() {
            return Some(false);
        }
        if self.remote.operator_life().is_some() {
            return None;
        }
        if self.remote.pending().is_none() {
            let Some(ticket) = self.streaks.reserve_radio(0, self.config.teams[0], id, now) else {
                return Some(false);
            };
            if !self.remote.begin(
                now,
                ticket,
                self.streaks.life(0),
                input.weapons[0],
                input.angles,
            ) {
                self.streaks.cancel_radio(0, ticket);
                return Some(false);
            }
        }
        if !self.remote.ready(now) {
            return None;
        }
        let p = self.remote.pending().unwrap();
        if p.ticket.native_id() != id {
            if let Some(ticket) = self.remote.cancel_pending() {
                self.streaks.cancel_radio(0, ticket);
            }
            return Some(false);
        }
        if matches!(id, 12 | 13) {
            let plan = self
                .remote
                .plan_gunner(r, id, self.aircraft.vehicle_slots(), c);
            if let Some(plan) = plan
                && self
                    .streaks
                    .commit_radio(0, self.config.teams[0], p.ticket, now)
            {
                self.remote.launch_gunner(
                    now,
                    plan,
                    Source {
                        owner: 0,
                        team: self.config.teams[0],
                        life: p.life,
                    },
                    r.tags["tag_origin"],
                    c,
                );
                return Some(true);
            }
            if let Some(ticket) = self.remote.cancel_pending() {
                self.streaks.cancel_radio(0, ticket);
            }
            return Some(false);
        }
        if id == 18 {
            let occupied = self
                .bots
                .iter()
                .filter(|b| b.health > 0)
                .map(|b| b.snapshot.origin)
                .collect::<Vec<_>>();
            let plan = self.remote.plan_car(
                r,
                self.config.teams[0],
                origin,
                p.angles,
                (&occupied, self.aircraft.vehicle_slots()),
                c,
            );
            if let Some(plan) = plan
                && self
                    .streaks
                    .commit_radio(0, self.config.teams[0], p.ticket, now)
            {
                self.remote.launch_car(
                    now,
                    plan,
                    Source {
                        owner: 0,
                        team: self.config.teams[0],
                        life: p.life,
                    },
                    r.tags["tag_player"],
                    c,
                );
                return Some(true);
            }
            if let Some(ticket) = self.remote.cancel_pending() {
                self.streaks.cancel_radio(0, ticket);
            }
            return Some(false);
        }
        let enemies = self
            .bots
            .iter()
            .filter(|b| b.health > 0 && b.snapshot.team != self.config.teams[0])
            .map(|b| b.snapshot.origin)
            .collect::<Vec<_>>();
        let plan = self.remote.plan(r, origin, p.angles, &enemies, c);
        if let Some(plan) = plan
            && self
                .streaks
                .commit_radio(0, self.config.teams[0], p.ticket, now)
        {
            self.remote.launch(
                now,
                plan,
                Source {
                    owner: 0,
                    team: self.config.teams[0],
                    life: p.life,
                },
            );
            return Some(true);
        }
        if let Some(ticket) = self.remote.cancel_pending() {
            self.streaks.cancel_radio(0, ticket);
        }
        Some(false)
    }
}
