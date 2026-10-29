//! Cooperative mission authority for the first story level.
//! Objectives advance from the player live position and kill notifications; a
//! death restarts the last checkpoint instead of the opening spawn.
mod mission;
#[cfg(test)]
mod tests;

pub(crate) use mission::Mission;

use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Completed { seconds: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    StageStarted { index: usize, id: String },
    Checkpoint { index: usize },
    Retried { attempts: u32 },
    Completed { seconds: u32 },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PlayerFrame {
    pub(crate) origin: [f32; 3],
    pub(crate) alive: bool,
}

pub(crate) struct StoryMatch {
    mission: Mission,
    stage: usize,
    checkpoint: usize,
    checkpoint_origin: [f32; 3],
    stage_start: u32,
    progress: u32,
    attempts: u32,
    started: Option<u32>,
    finished: Option<u32>,
    events: Vec<Event>,
}

impl StoryMatch {
    pub(crate) fn new(mission: Mission) -> Result<Self> {
        mission.validate()?;
        let checkpoint_origin = mission.spawn.origin;
        Ok(Self {
            mission,
            stage: 0,
            checkpoint: 0,
            checkpoint_origin,
            stage_start: 0,
            progress: 0,
            attempts: 0,
            started: None,
            finished: None,
            events: Vec::new(),
        })
    }

    pub(crate) fn mission(&self) -> &Mission {
        &self.mission
    }

    pub(crate) fn spawn_origin(&self) -> [f32; 3] {
        self.mission.spawn.origin
    }

    pub(crate) fn spawn_angles(&self) -> [u16; 3] {
        self.mission.spawn.angles
    }

    /// Origin the next attempt restarts from.
    pub(crate) fn respawn_origin(&self) -> [f32; 3] {
        self.checkpoint_origin
    }

    pub(crate) fn stage_index(&self) -> usize {
        self.stage
    }

    pub(crate) fn stage_id(&self) -> &str {
        &self.mission.stages[self.stage].id
    }

    pub(crate) fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(crate) fn outcome(&self) -> Option<Outcome> {
        self.finished.map(|finished| Outcome::Completed {
            seconds: finished.saturating_sub(self.started.unwrap_or(finished)) / 1000,
        })
    }

    pub(crate) fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Elimination progress comes from authoritative combat kills, never from a
    /// client-reported counter.
    /// The connection reports a kill when the captured destructible target
    /// dies, which is the only enemy health this build simulates.
    pub(crate) fn record_kill(&mut self) {
        if self.finished.is_some() {
            return;
        }
        if matches!(
            self.mission.stages[self.stage].objective,
            mission::Objective::Eliminate { .. }
        ) {
            self.progress = self.progress.saturating_add(1);
        }
    }

    pub(crate) fn advance(&mut self, now: u32, frame: PlayerFrame) {
        if self.finished.is_some() {
            return;
        }
        if self.started.is_none() {
            self.started = Some(now);
            self.stage_start = now;
            self.events.push(Event::StageStarted {
                index: 0,
                id: self.mission.stages[0].id.clone(),
            });
            return;
        }
        if !frame.alive {
            self.attempts = self.attempts.saturating_add(1);
            self.stage = self.checkpoint;
            self.stage_start = now;
            self.progress = 0;
            self.events.push(Event::Retried {
                attempts: self.attempts,
            });
            return;
        }
        loop {
            let objective = self.mission.stages[self.stage].objective;
            let complete = match objective {
                mission::Objective::Reach { origin, radius }
                | mission::Objective::Finish { origin, radius } => {
                    inside(frame.origin, origin, radius)
                }
                mission::Objective::Eliminate { count } => self.progress >= u32::from(count),
                mission::Objective::Survive { seconds } => {
                    let elapsed = now.saturating_sub(self.stage_start);
                    self.progress = elapsed;
                    elapsed >= u32::from(seconds).saturating_mul(1000)
                }
            };
            if !complete {
                return;
            }
            let finished = self.stage;
            if matches!(objective, mission::Objective::Finish { .. }) {
                self.finished = Some(now);
                self.events.push(Event::Completed {
                    seconds: now.saturating_sub(self.started.unwrap_or(now)) / 1000,
                });
                return;
            }
            let next = finished + 1;
            if self.mission.stages[finished].checkpoint {
                self.checkpoint = next;
                self.checkpoint_origin = self.stage_origin(next);
                self.events.push(Event::Checkpoint { index: next });
            }
            self.stage = next;
            self.stage_start = now;
            self.progress = 0;
            self.events.push(Event::StageStarted {
                index: next,
                id: self.mission.stages[next].id.clone(),
            });
        }
    }

    fn stage_origin(&self, index: usize) -> [f32; 3] {
        match self.mission.stages[index].objective {
            mission::Objective::Reach { origin, .. }
            | mission::Objective::Finish { origin, .. } => origin,
            _ => self.mission.spawn.origin,
        }
    }
}

fn inside(point: [f32; 3], origin: [f32; 3], radius: f32) -> bool {
    let dx = point[0] - origin[0];
    let dy = point[1] - origin[1];
    let dz = point[2] - origin[2];
    dx * dx + dy * dy + dz * dz <= radius * radius
}
