//! Structural diagnostics for exported movement captures, not physics acceptance.
//!
//! Run the ignored diagnostic explicitly with both COOL_REPLAY and COOL_COLLISION.
//! The current exporter does not reconstruct every command's angle baseline or
//! capture a complete initial Motion state. Prediction-anchored results therefore
//! remain untrusted even when an observed positional error happens to be small.

use anyhow::{Context, Result, ensure};
use codol_protocol::game::channel::usercmd::UserCommand;
use codol_server::game::player::movement::Motion;
use codol_server::game::world::collision::Collision;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
struct Record {
    session: u32,
    #[serde(default)]
    sequence: Option<u32>,
    commands: Vec<Cmd>,
    predicted_origin: [f32; 3],
    predicted_time: u32,
    /// Optional explicit reset/teleport marker from an enriched exporter.
    #[serde(default)]
    reset: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Cmd {
    time: u32,
    buttons: u32,
    angles: [u16; 3],
    direction: u8,
    weapons: [u16; 3],
}

impl From<&Cmd> for UserCommand {
    fn from(c: &Cmd) -> Self {
        Self {
            time: c.time,
            buttons: c.buttons,
            angles: c.angles,
            direction: c.direction,
            weapons: c.weapons,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Boundary {
    SessionStart,
    ExplicitReset,
    TimeRegression,
    CommandGap,
    PositionDiscontinuity,
}

#[derive(Debug)]
struct Segment {
    boundary: Boundary,
    anchor_time: u32,
    anchor_origin: [f32; 3],
    commands: BTreeMap<u32, Cmd>,
    predictions: BTreeMap<u32, Option<[f32; 3]>>,
    retransmits: usize,
    ambiguous_predictions: usize,
}

#[derive(Debug)]
struct SessionPlan {
    session: u32,
    records: usize,
    segments: Vec<Segment>,
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum::<f64>()
        .sqrt()
}

// These detect discontinuities to label and re-anchor; neither is an acceptable
// physics error bound. A 1000 ms gap exceeds the target Pmove catch-up window.
const COMMAND_GAP_MS: u32 = 1000;
const POSITION_DISCONTINUITY: f64 = 200.0;

fn plans(records: &[Record]) -> Result<Vec<SessionPlan>> {
    ensure!(!records.is_empty(), "replay capture is empty");
    let mut groups = BTreeMap::<u32, Vec<&Record>>::new();
    for (index, record) in records.iter().enumerate() {
        ensure!(
            record.session != 0,
            "record {} has no valid session",
            index + 1
        );
        ensure!(
            (1..=i32::MAX as u32).contains(&record.predicted_time)
                && record.predicted_origin.iter().all(|v| v.is_finite()),
            "record {} has invalid prediction state",
            index + 1
        );
        ensure!(
            record.commands.windows(2).all(|c| c[0].time <= c[1].time),
            "session {} record {} commands are not chronological",
            record.session,
            index + 1
        );
        for command in &record.commands {
            ensure!(
                (1..=i32::MAX as u32).contains(&command.time) && command.direction < 16,
                "session {} record {} has invalid input",
                record.session,
                index + 1
            );
        }
        groups.entry(record.session).or_default().push(record);
    }
    let mut output = Vec::new();
    for (session, records) in groups {
        // Retain capture order: an explicit time reset must remain a boundary.
        // Sorting every command globally would merge epochs with reused times.
        let mut plan = SessionPlan {
            session,
            records: records.len(),
            segments: vec![],
        };
        let mut previous: Option<&Record> = None;
        let mut epoch_commands = BTreeMap::<u32, Cmd>::new();
        for record in records {
            let boundary = match previous {
                None => Some(Boundary::SessionStart),
                Some(_) if record.reset => Some(Boundary::ExplicitReset),
                Some(p) if record.predicted_time < p.predicted_time => {
                    Some(Boundary::TimeRegression)
                }
                Some(p) if record.predicted_time - p.predicted_time > COMMAND_GAP_MS => {
                    Some(Boundary::CommandGap)
                }
                Some(p)
                    if record.predicted_time > p.predicted_time
                        && distance(record.predicted_origin, p.predicted_origin)
                            > POSITION_DISCONTINUITY =>
                {
                    Some(Boundary::PositionDiscontinuity)
                }
                Some(_) => None,
            };
            if matches!(
                boundary,
                Some(Boundary::ExplicitReset | Boundary::TimeRegression)
            ) {
                epoch_commands.clear();
            }
            if let Some(boundary) = boundary {
                plan.segments.push(Segment {
                    boundary,
                    anchor_time: record.predicted_time,
                    anchor_origin: record.predicted_origin,
                    commands: BTreeMap::new(),
                    predictions: BTreeMap::new(),
                    retransmits: 0,
                    ambiguous_predictions: 0,
                });
            }
            let segment_index = plan.segments.len() - 1;
            let segment = &mut plan.segments[segment_index];
            for command in &record.commands {
                // A positional teleport does not change the input clock.
                // Check retransmits across those segment boundaries as well.
                if let Some(old) = epoch_commands.get(&command.time) {
                    ensure!(
                        old == command,
                        "session {session} segment {segment_index} conflicting command at time {} (packet {:?}); angle baselines and all input values must agree before replay",
                        command.time,
                        record.sequence
                    );
                } else {
                    epoch_commands.insert(command.time, command.clone());
                }
                if let Some(old) = segment.commands.get(&command.time) {
                    ensure!(
                        old == command,
                        "session {session} segment {segment_index} conflicting command at time {} (packet {:?}); angle baselines and all input values must agree before replay",
                        command.time,
                        record.sequence
                    );
                    segment.retransmits += 1;
                } else {
                    segment.commands.insert(command.time, command.clone());
                }
            }
            match segment.predictions.entry(record.predicted_time) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(Some(record.predicted_origin));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry
                        .get()
                        .is_some_and(|old| old != record.predicted_origin)
                    {
                        // A new acknowledged snapshot may correct the same
                        // predicted time. Retain ambiguity instead of letting
                        // the last packet silently replace the earlier origin.
                        entry.insert(None);
                        segment.ambiguous_predictions += 1;
                    }
                }
            }
            previous = Some(record);
        }
        output.push(plan);
    }
    Ok(output)
}

#[derive(Debug, Serialize)]
struct SegmentReport {
    boundary: Boundary,
    anchor_time: u32,
    initial_state: &'static str,
    heading: &'static str,
    applied_commands: usize,
    retransmits: usize,
    samples: usize,
    unmatched_prediction_times: usize,
    ambiguous_prediction_times: usize,
    mean_distance: Option<f64>,
    worst_distance: Option<f64>,
}

#[derive(Debug, Serialize)]
struct SessionReport {
    session: u32,
    records: usize,
    physics_verified: bool,
    assumptions: [&'static str; 3],
    samples: usize,
    mean_distance: Option<f64>,
    worst_distance: Option<f64>,
    segments: Vec<SegmentReport>,
}

fn seeded_motion(origin: [f32; 3], time: u32) -> Motion {
    let mut motion = Motion::new(origin);
    // The prediction anchor is already a state *at* this input time. Replaying
    // that command from time zero would apply up to 1000 ms of invented motion.
    motion.command_time = time;
    motion
}

fn evaluate(
    plan: &SessionPlan,
    mut advance: impl FnMut(&mut Motion, &UserCommand),
) -> Result<SessionReport> {
    let mut reports = Vec::new();
    let mut all_distances = Vec::new();
    for segment in &plan.segments {
        let mut motion = seeded_motion(segment.anchor_origin, segment.anchor_time);
        let mut distances = Vec::new();
        let mut applied = 0;
        for command in segment
            .commands
            .values()
            .filter(|c| c.time > segment.anchor_time)
        {
            advance(&mut motion, &UserCommand::from(command));
            ensure!(
                motion.command_time == command.time,
                "session {} motion did not reach input time {}",
                plan.session,
                command.time
            );
            ensure!(
                motion.origin.iter().all(|v| v.is_finite()),
                "session {} motion produced a non-finite origin",
                plan.session
            );
            applied += 1;
            if let Some(Some(target)) = segment.predictions.get(&command.time) {
                distances.push(distance(motion.origin, *target));
            }
        }
        let unmatched = segment
            .predictions
            .iter()
            .filter(|(time, target)| {
                **time > segment.anchor_time
                    && target.is_some()
                    && !segment.commands.contains_key(time)
            })
            .count();
        let samples = distances.len();
        let mean = (!distances.is_empty()).then(|| distances.iter().sum::<f64>() / samples as f64);
        let worst = distances.iter().copied().reduce(f64::max);
        all_distances.extend(distances);
        reports.push(SegmentReport {
            boundary: segment.boundary,
            anchor_time: segment.anchor_time,
            initial_state: "prediction_origin_only_missing_velocity_ground_flags_and_prior_input",
            heading: "not_verified_against_acknowledged_angle_baseline",
            applied_commands: applied,
            retransmits: segment.retransmits,
            samples,
            unmatched_prediction_times: unmatched,
            ambiguous_prediction_times: segment.ambiguous_predictions,
            mean_distance: mean,
            worst_distance: worst,
        });
    }
    let samples = all_distances.len();
    Ok(SessionReport {
        session: plan.session,
        records: plan.records,
        physics_verified: false,
        assumptions: [
            "initial_and_reset_states_are_incomplete_prediction_anchors",
            "captured_angles_are_not_known_to_have_complete_delta_baselines",
            "weapon_scale_1_and_ads_false_are_diagnostic_assumptions",
        ],
        samples,
        mean_distance: (!all_distances.is_empty())
            .then(|| all_distances.iter().sum::<f64>() / samples as f64),
        worst_distance: all_distances.into_iter().reduce(f64::max),
        segments: reports,
    })
}

fn explicit_paths(
    replay: Option<OsString>,
    collision: Option<OsString>,
) -> Result<(PathBuf, PathBuf)> {
    let replay = replay.context("set COOL_REPLAY to the exported JSONL capture")?;
    let collision = collision.context("set COOL_COLLISION to the matching collision export")?;
    ensure!(
        !replay.is_empty() && !collision.is_empty(),
        "replay and collision paths must not be empty"
    );
    Ok((replay.into(), collision.into()))
}

fn read_records(path: &Path) -> Result<Vec<Record>> {
    const LIMIT: u64 = 64 * 1024 * 1024;
    let file = File::open(path).with_context(|| format!("open replay {}", path.display()))?;
    let mut text = String::new();
    file.take(LIMIT + 1).read_to_string(&mut text)?;
    ensure!(text.len() as u64 <= LIMIT, "replay exceeds 64 MiB");
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("replay {} line {}", path.display(), i + 1))
        })
        .collect()
}

#[test]
#[ignore = "diagnostic only; requires explicit COOL_REPLAY and COOL_COLLISION, then --ignored --nocapture"]
fn captured_movement_reports_untrusted_divergence_per_session() -> Result<()> {
    let (replay, export) = explicit_paths(
        std::env::var_os("COOL_REPLAY"),
        std::env::var_os("COOL_COLLISION"),
    )?;
    let records = read_records(&replay)?;
    let plans = plans(&records)?;
    let collision =
        Collision::load(&export).with_context(|| format!("load collision {}", export.display()))?;
    for plan in plans {
        let report = evaluate(&plan, |motion, cmd| {
            motion.apply(cmd, 1.0, false, &collision)
        })?;
        eprintln!("MOVEMENT_DIAGNOSTIC {}", serde_json::to_string(&report)?);
    }
    Ok(())
}

fn command(time: u32) -> Cmd {
    Cmd {
        time,
        buttons: 0,
        angles: [0; 3],
        direction: 1,
        weapons: [1, 0, 0],
    }
}

fn record(session: u32, time: u32, x: f32, commands: Vec<Cmd>) -> Record {
    Record {
        session,
        sequence: None,
        commands,
        predicted_origin: [x, 0.0, 0.0],
        predicted_time: time,
        reset: false,
    }
}

// Synthetic motion isolates replay ordering/time pairing from collision assets.
fn linear_motion(motion: &mut Motion, cmd: &UserCommand) {
    motion.origin[0] += (cmd.time - motion.command_time) as f32 * 0.001;
    motion.command_time = cmd.time;
}

#[test]
fn interleaved_sessions_keep_same_timestamps_and_positions_separate() {
    let input = [
        record(1, 1000, 10.0, vec![command(1000)]),
        record(2, 1000, 100.0, vec![command(1000)]),
        record(1, 1100, 10.1, vec![command(1100)]),
        record(2, 1100, 100.1, vec![command(1100)]),
    ];
    let plans = plans(&input).unwrap();
    assert_eq!(plans.len(), 2);
    for plan in &plans {
        let report = evaluate(plan, linear_motion).unwrap();
        assert_eq!(report.samples, 1);
        assert!(report.mean_distance.unwrap() < 0.00001);
        assert!(!report.physics_verified);
    }
}

#[test]
fn anchors_and_teleports_initialize_time_and_are_not_reexecuted() {
    let input = [
        record(1, 10_000, 1.0, vec![command(9990), command(10_000)]),
        record(1, 10_100, 1.1, vec![command(10_100)]),
        record(1, 10_200, 1000.0, vec![command(10_200)]),
        record(1, 10_300, 1000.1, vec![command(10_300)]),
    ];
    let plan = plans(&input).unwrap().remove(0);
    assert_eq!(plan.segments[1].boundary, Boundary::PositionDiscontinuity);
    let mut steps = Vec::new();
    let report = evaluate(&plan, |motion, cmd| {
        steps.push((motion.command_time, cmd.time));
        linear_motion(motion, cmd);
    })
    .unwrap();
    assert_eq!(steps, [(10_000, 10_100), (10_200, 10_300)]);
    assert_eq!(report.samples, 2);
    assert!(report.worst_distance.unwrap() < 0.0001);
}

#[test]
fn explicit_reset_starts_a_new_epoch_even_when_times_are_reused() {
    let mut reset = record(1, 1000, 5.0, vec![command(1000)]);
    reset.reset = true;
    reset.commands[0].buttons = 0x400;
    let input = [
        record(1, 1000, 0.0, vec![command(1000)]),
        record(1, 1100, 0.1, vec![command(1100)]),
        reset,
        record(1, 1100, 5.1, vec![command(1100)]),
    ];
    let plan = plans(&input).unwrap().remove(0);
    assert_eq!(plan.segments.len(), 2);
    assert_eq!(plan.segments[1].boundary, Boundary::ExplicitReset);
    let report = evaluate(&plan, linear_motion).unwrap();
    assert_eq!(report.samples, 2);
    assert!(report.worst_distance.unwrap() < 0.00001);
}

#[test]
fn retransmits_are_deduplicated_and_changed_values_are_rejected() {
    let mut input = vec![
        record(1, 1000, 0.0, vec![command(1000)]),
        record(1, 1100, 0.1, vec![command(1000), command(1100)]),
        record(1, 1100, 0.1, vec![command(1100)]),
    ];
    let plan = plans(&input).unwrap().remove(0);
    let report = evaluate(&plan, linear_motion).unwrap();
    assert_eq!(report.segments[0].applied_commands, 1);
    assert_eq!(report.segments[0].retransmits, 2);
    assert_eq!(report.samples, 1);
    input[2].commands[0].angles[1] = 90;
    let error = plans(&input).unwrap_err().to_string();
    assert!(error.contains("session 1") && error.contains("conflicting command at time 1100"));
}

#[test]
fn position_reset_does_not_hide_conflicting_old_input() {
    let mut changed = command(1000);
    changed.buttons = 0x400;
    let input = [
        record(1, 1000, 0.0, vec![command(1000)]),
        record(1, 1100, 1000.0, vec![changed, command(1100)]),
    ];
    assert!(
        plans(&input)
            .unwrap_err()
            .to_string()
            .contains("conflicting command at time 1000")
    );
}

#[test]
fn prediction_times_must_match_and_conflicting_positions_are_not_overwritten() {
    let input = [
        record(1, 1000, 0.0, vec![command(1000)]),
        record(1, 1100, 0.1, vec![command(1100)]),
        record(1, 1100, 0.2, vec![command(1100)]),
        record(1, 1150, 0.15, vec![]),
        record(1, 1200, 0.2, vec![command(1200)]),
    ];
    let plan = plans(&input).unwrap().remove(0);
    let report = evaluate(&plan, linear_motion).unwrap();
    assert_eq!(report.samples, 1);
    assert_eq!(report.segments[0].ambiguous_prediction_times, 1);
    assert_eq!(report.segments[0].unmatched_prediction_times, 1);
    assert!(report.mean_distance.unwrap() < 0.00001);
}

#[test]
fn time_regressions_and_uncovered_gaps_are_visible_boundaries() {
    let input = [
        record(1, 1000, 0.0, vec![command(1000)]),
        record(1, 1100, 0.1, vec![command(1100)]),
        record(1, 1000, 0.0, vec![command(1000)]),
        record(1, 3000, 5.0, vec![command(3000)]),
        record(1, 3100, 5.1, vec![command(3100)]),
    ];
    let plan = plans(&input).unwrap().remove(0);
    assert_eq!(
        plan.segments.iter().map(|s| s.boundary).collect::<Vec<_>>(),
        [
            Boundary::SessionStart,
            Boundary::TimeRegression,
            Boundary::CommandGap
        ]
    );
    let report = evaluate(&plan, linear_motion).unwrap();
    assert_eq!(report.samples, 2);
}

#[test]
fn explicit_missing_paths_and_missing_sessions_fail_instead_of_skipping() {
    assert!(explicit_paths(None, None).is_err());
    assert!(explicit_paths(Some("capture".into()), None).is_err());
    let temp = tempfile::tempdir().unwrap();
    assert!(read_records(&temp.path().join("missing.jsonl")).is_err());
    assert!(Collision::load(&temp.path().join("missing-collision.json")).is_err());
    let data = r#"{"commands":[],"predicted_origin":[0,0,0],"predicted_time":1000}"#;
    assert!(serde_json::from_str::<Record>(data).is_err());
    assert!(plans(&[]).is_err());
}
