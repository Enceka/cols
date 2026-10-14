//! Exercise room creation, browsing and admission against isolated local services.
//! Uses synthetic authenticated identities; does not operate the live client/backend.
use anyhow::{Context, Result, ensure};
use codol_protocol::lobby::task;
use codol_protocol::lobby::task::Reader;
use codol_protocol::lobby::task::Request;
use codol_protocol::lobby::task::Writer;
use codol_protocol::room::browser as room_browser;
use codol_protocol::room::description::Description;
use codol_server::app::services::LocalServices;
use codol_server::backend::identity::session::Session;
use codol_server::transport::game::server::GameServer;
use serde_json::{Value, json};
use std::path::PathBuf;

fn bytes(value: &Value) -> Result<Vec<u8>> {
    let hex = value.as_str().context("fixture must contain hex")?;
    ensure!(hex.len().is_multiple_of(2), "odd hex length");
    (0..hex.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&hex[i..i + 2], 16)?))
        .collect()
}

fn session(user_id: u64) -> Session {
    Session {
        title_id: 1002,
        user_id,
        iv_seed: user_id as u32,
        expires_at: u32::MAX,
        key: [0; 24],
    }
}

async fn observe(services: &LocalServices, body: &[u8], actor: &Session) -> Value {
    match services.handle(body, actor, 100).await {
        Ok(response) => json!({
            "handler": response.label,
            "error_code": response.result.as_ref().err().copied().unwrap_or(0),
            "result_count": response.result.as_ref().map_or(0, Vec::len),
            "push_count": response.pushes.len(),
        }),
        // The lobby transport maps a handler error to this failure acknowledgement.
        Err(error) => json!({
            "handler": "service_decode_failure",
            "error_code": task::TASK_FAILED,
            "result_count": 0,
            "decode_error": error.to_string(),
        }),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("usage: room_browser_acceptance OUTPUT_JSON")?,
    );
    let temp = tempfile::tempdir()?;
    // An ephemeral UDP socket enables the same allocation guard as the live backend.
    let game = GameServer::bind("127.0.0.1:0".parse()?, temp.path().join("game")).await?;
    let services = LocalServices::open(temp.path().join("services"))
        .await?
        .with_game_endpoint(game.admissions())
        .await;
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-browser-synthetic.json"
    )))?;
    let owner = session(42);
    let guest = session(43);

    let channel = services.handle(&[11, 3, 13, 0], &owner, 100).await?;
    let records = channel
        .result
        .map_err(|code| anyhow::anyhow!("create channel failed: {code}"))?;
    let channel_id = Reader::new(records.first().context("channel record missing")?).u64()?;
    let mut descriptor = bytes(&fixture["listing"]["create_arguments"])?;
    // Native tail: UInt64 channel, two Int32 attributes, then NoType.
    let start = descriptor
        .len()
        .checked_sub(20)
        .context("short descriptor")?;
    ensure!(descriptor[start] == 10, "channel tail tag changed");
    descriptor[start + 1..start + 9].copy_from_slice(&channel_id.to_le_bytes());
    ensure!(
        Description::parse(&descriptor, owner.user_id)?.channel_id == channel_id,
        "channel reference mismatch"
    );
    let mut create = vec![21, 3, 1];
    create.extend(descriptor);
    let created = observe(&services, &create, &owner).await;
    ensure!(
        created["error_code"] == 0,
        "baseline session creation failed"
    );
    let mut cases = vec![json!({
        "name": "owner_registers_room",
        "passed": true,
        "actual": created,
    })];

    for name in ["ordinary", "first_page"] {
        let body = bytes(&fixture["requests"][name])?;
        room_browser::Search::parse(&Request::parse(&body)?)?;
        let actual = observe(&services, &body, &guest).await;
        let passed = actual["error_code"] == 0 && actual["result_count"] == 1;
        cases.push(json!({
            "name": format!("guest_browses_{name}"),
            "request_codec_passed": true,
            "expected": "successful query containing the owner's registered room",
            "passed": passed,
            "actual": actual,
        }));
    }

    // Native 3.3.3.3 MP browsing can send criterion 16 with a page size of 30.
    // Keep its unsupported result visible alongside the synthetic default query.
    let captured_query = bytes(&json!(
        "15030d080400000001011308080000000101010101010101081e000000079b1ff0d607000000000710000000070000000000"
    ))?;
    room_browser::Search::parse(&Request::parse(&captured_query)?)?;
    let actual = observe(&services, &captured_query, &guest).await;
    cases.push(json!({
        "name": "native_observed_browser_criterion16_is_not_supported",
        "expected": "unsupported_browser_criteria with TASK_FAILED",
        "passed": actual["handler"] == "unsupported_browser_criteria"
            && actual["error_code"] == task::TASK_FAILED,
        "actual": actual,
    }));

    let mut args = Writer::default();
    args.u64(channel_id);
    args.u64(0);
    args.0.push(0);
    room_browser::ChannelJoin::parse(&args.0)?;
    let mut join = vec![11, 3, 11];
    join.extend(args.0);
    let actual = observe(&services, &join, &guest).await;
    cases.push(json!({
        "name": "guest_joins_registered_channel",
        "request_codec_passed": true,
        "expected": "successful channel admission for the second identity",
        "passed": actual["error_code"] == 0,
        "actual": actual,
    }));
    let mut allocation = Writer(vec![21, 3, 5]);
    allocation.u32(1);
    allocation.u32(0);
    allocation.u32(10);
    allocation.i32(-688906341);
    allocation.0.push(0);
    let actual = observe(&services, &allocation.0, &owner).await;
    cases.push(json!({
        "name": "multiple_humans_cannot_allocate_a_shared_match",
        "expected": "shared_game_world_not_available with TASK_FAILED",
        "passed": actual["handler"] == "shared_game_world_not_available"
            && actual["error_code"] == task::TASK_FAILED,
        "actual": actual,
    }));
    let loadout = codol_protocol::room::messages::Loadout {
        clothing: [1; 12],
        weapons: [[1; 7]; 2],
        equipment: [0; 6],
        perks: [0; 3],
        rewards: [0; 3],
        trailing_item: 0,
    };
    let rejected =
        codol_protocol::room::messages::validate_participant(&loadout.record(1)?, owner.user_id)
            .err()
            .map(|error| error.to_string());
    cases.push(json!({
        "name": "participant_loadout_type18_is_not_supported",
        "expected": "participant cannot produce this room command",
        "passed": rejected.as_deref() == Some("participant cannot produce this room command"),
        "actual": rejected,
    }));
    let passed = cases.iter().all(|case| case["passed"] == true);
    let report = json!({
        "schema_version": 1,
        "scope": "isolated_authenticated_local_service_dispatch_not_native_ui_acceptance",
        "client_sha256": codol_protocol::CLIENT_SHA256,
        "game_endpoint_enabled": true,
        "synthetic_distinct_identities": true,
        "live_backend_or_account_data_modified": false,
        "multiplayer_acceptance_passed": false,
        "passed": passed,
        "cases": cases,
    });
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string(&report)?);
    drop(services);
    drop(game);
    drop(temp);
    if !passed {
        std::process::exit(1);
    }
    Ok(())
}
