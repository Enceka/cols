use super::*;
use crate::game::modes::story::mission::{Completion, Objective, Spawn, Stage};

fn stage(id: &str, checkpoint: bool, objective: Objective) -> Stage {
    Stage {
        id: id.into(),
        checkpoint,
        objective,
    }
}

fn mission() -> Mission {
    Mission {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        map: codol_protocol::room::story::FIRST_MAP.into(),
        mode: "coop".into(),
        spawn: Spawn {
            origin: [0.0, 0.0, 40.0],
            angles: [0, 90, 0],
        },
        stages: vec![
            stage(
                "reach_deck",
                true,
                Objective::Reach {
                    origin: [512.0, 0.0, 40.0],
                    radius: 128.0,
                },
            ),
            stage("clear_deck", false, Objective::Eliminate { count: 2 }),
            stage("hold_deck", false, Objective::Survive { seconds: 3 }),
            stage(
                "extract",
                false,
                Objective::Finish {
                    origin: [-512.0, 0.0, 40.0],
                    radius: 128.0,
                },
            ),
        ],
        completion: Completion {
            unlock_flag: codol_protocol::room::story::FIRST_COMPLETION_FLAG,
            native_flag: codol_protocol::room::story::FIRST_COMPLETION_NAME.into(),
        },
    }
}

fn frame(origin: [f32; 3], alive: bool) -> PlayerFrame {
    PlayerFrame { origin, alive }
}

#[test]
fn objectives_advance_from_position_kills_and_time() {
    let mut game = StoryMatch::new(mission()).unwrap();
    game.advance(1000, frame([0.0, 0.0, 40.0], true));
    assert_eq!(
        game.take_events(),
        vec![Event::StageStarted {
            index: 0,
            id: "reach_deck".into()
        }]
    );
    game.advance(1500, frame([0.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 0);
    game.advance(2000, frame([512.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 1);
    assert_eq!(
        game.take_events(),
        vec![
            Event::Checkpoint { index: 1 },
            Event::StageStarted {
                index: 1,
                id: "clear_deck".into()
            },
        ]
    );
    game.record_kill();
    game.advance(2100, frame([512.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 1);
    game.record_kill();
    game.advance(2200, frame([512.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 2);
    game.advance(5100, frame([0.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 2);
    game.advance(5200, frame([0.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 3);
    game.advance(6000, frame([-512.0, 0.0, 40.0], true));
    assert_eq!(game.outcome(), Some(Outcome::Completed { seconds: 5 }));
    assert!(
        game.take_events()
            .contains(&Event::Completed { seconds: 5 })
    );
}

fn checkpointed() -> Mission {
    let mut mission = mission();
    mission.stages = vec![
        stage(
            "reach_bow",
            true,
            Objective::Reach {
                origin: [100.0, 0.0, 40.0],
                radius: 64.0,
            },
        ),
        stage(
            "reach_stern",
            true,
            Objective::Reach {
                origin: [-100.0, 0.0, 40.0],
                radius: 64.0,
            },
        ),
        stage(
            "extract",
            false,
            Objective::Finish {
                origin: [0.0, 0.0, 40.0],
                radius: 64.0,
            },
        ),
    ];
    mission
}

#[test]
fn death_restarts_the_last_checkpoint_and_counts_attempts() {
    let mut game = StoryMatch::new(checkpointed()).unwrap();
    game.advance(1000, frame([0.0, 0.0, 40.0], true));
    assert_eq!(game.respawn_origin(), [0.0, 0.0, 40.0]);
    game.advance(2000, frame([100.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 1);
    assert_eq!(game.respawn_origin(), [-100.0, 0.0, 40.0]);
    game.advance(3000, frame([100.0, 0.0, 40.0], false));
    assert_eq!(game.attempts(), 1);
    assert_eq!(game.stage_index(), 1);
    assert_eq!(game.respawn_origin(), [-100.0, 0.0, 40.0]);
    let retry_events = game.take_events();
    assert_eq!(retry_events.last(), Some(&Event::Retried { attempts: 1 }));
    game.advance(4000, frame([-100.0, 0.0, 40.0], true));
    assert_eq!(game.stage_index(), 2);
    assert_eq!(game.respawn_origin(), [0.0, 0.0, 40.0]);
    game.advance(5000, frame([-100.0, 0.0, 40.0], false));
    assert_eq!(game.attempts(), 2);
    assert_eq!(game.stage_index(), 2);
    game.advance(6000, frame([0.0, 0.0, 40.0], true));
    assert_eq!(game.outcome(), Some(Outcome::Completed { seconds: 5 }));
    game.advance(9000, frame([0.0, 0.0, 40.0], false));
    assert_eq!(game.attempts(), 2);
}

#[test]
fn mission_definition_rejects_mismatched_identity_and_flags() {
    let mut wrong_map = mission();
    wrong_map.map = "coop_take_mp_fav_tropical".into();
    assert!(wrong_map.validate().is_err());
    let mut wrong_mode = mission();
    wrong_mode.mode = "war".into();
    assert!(wrong_mode.validate().is_err());
    let mut wrong_flag = mission();
    wrong_flag.completion.unlock_flag = 151;
    assert!(wrong_flag.validate().is_err());
    let mut wrong_name = mission();
    wrong_name.completion.native_flag = "cf_ch_coop_take".into();
    assert!(wrong_name.validate().is_err());
    let mut wrong_client = mission();
    wrong_client.client_sha256 = "0".repeat(64);
    assert!(wrong_client.validate().is_err());
}

#[test]
fn mission_definition_requires_canonical_stages_and_a_final_finish() {
    let mut no_finish = mission();
    no_finish.stages.pop();
    assert!(no_finish.validate().is_err());
    let mut early_finish = mission();
    early_finish.stages.swap(0, 3);
    assert!(early_finish.validate().is_err());
    let mut duplicate = mission();
    duplicate.stages[1].id = "reach_deck".into();
    assert!(duplicate.validate().is_err());
    let mut uppercase = mission();
    uppercase.stages[1].id = "Clear_Deck".into();
    assert!(uppercase.validate().is_err());
    let mut tiny = mission();
    tiny.stages[0].objective = Objective::Reach {
        origin: [512.0, 0.0, 40.0],
        radius: 8.0,
    };
    assert!(tiny.validate().is_err());
    let mut far = mission();
    far.spawn.origin = [1.0e6, 0.0, 0.0];
    assert!(far.validate().is_err());
}

#[test]
fn mission_file_loads_and_rejects_oversized_input() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&mission()).unwrap()).unwrap();
    let loaded = Mission::load(file.path()).unwrap();
    assert_eq!(loaded.stages.len(), 4);
    let oversized = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(oversized.path(), vec![0u8; 70 * 1024]).unwrap();
    assert!(Mission::load(oversized.path()).is_err());
    let invalid = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(invalid.path(), b"{}").unwrap();
    assert!(Mission::load(invalid.path()).is_err());
}

#[test]
fn first_level_fixture_loads_and_ends_with_a_finish_stage() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/protocol/story-first-mission-synthetic.json");
    let mission = Mission::load(&path).unwrap();
    assert_eq!(mission.map, codol_protocol::room::story::FIRST_MAP);
    assert_eq!(mission.stages.len(), 4);
    assert_eq!(mission.stages[0].id, "onboard");
    assert_eq!(mission.stages[3].id, "debrief");
    assert!(matches!(
        mission.stages[3].objective,
        Objective::Finish { .. }
    ));
    assert!(mission.stages[1].checkpoint && mission.stages[2].checkpoint);
}
