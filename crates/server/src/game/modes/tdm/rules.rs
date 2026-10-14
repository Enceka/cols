//! TDM scoring and terminal-state rules, without entities or network access.
#[derive(Default)]
pub(super) struct Rules {
    pub(super) forced_winner: Option<u8>,
    pub(super) scores: [i32; 4],
    pub(super) finished: bool,
    pub(super) finished_at: Option<u32>,
    pub(super) start: Option<u32>,
}
impl Rules {
    pub fn result(&self) -> &'static str {
        if let Some(team) = self.forced_winner {
            return if team == 2 { "Allies win" } else { "Axis win" };
        }
        match self.scores[2].cmp(&self.scores[1]) {
            std::cmp::Ordering::Greater => "Allies win",
            std::cmp::Ordering::Less => "Axis win",
            std::cmp::Ordering::Equal => "Draw",
        }
    }
    pub fn winning_team(&self) -> u8 {
        if let Some(team) = self.forced_winner {
            return team;
        }
        match self.scores[2].cmp(&self.scores[1]) {
            std::cmp::Ordering::Greater => 2,
            std::cmp::Ordering::Less => 1,
            std::cmp::Ordering::Equal => 0,
        }
    }
    pub fn duration_seconds(&self) -> u32 {
        self.finished_at
            .unwrap_or(0)
            .saturating_sub(self.start.unwrap_or(0))
            / 1000
    }
    pub(super) fn time_expired(&mut self, now: u32, limit: u32, incoming_reward: bool) -> bool {
        if self.finished {
            return false;
        }
        let start = *self.start.get_or_insert(now);
        !incoming_reward && now.saturating_sub(start) >= limit
    }
    pub(super) fn score_reached(&self, team: usize, limit: u32, incoming_reward: bool) -> bool {
        !incoming_reward && self.scores[team] >= limit as i32
    }
}
