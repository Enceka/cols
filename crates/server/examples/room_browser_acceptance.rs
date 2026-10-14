//! Exercise room creation, browsing and admission against isolated local services.
//! Uses synthetic authenticated identities; does not operate the live client/backend.
use anyhow::{Context, Result, ensure};
use codol_protocol::{
    room::Description,
    room_browser,
    task::{self, Reader, Request, Writer},
};
use codol_server::{game_server::GameServer, services::LocalServices, session::Session};
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
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/room-browser-synthetic.json"
    ))?;
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
    let passed = cases.iter().all(|case| case["passed"] == true);
    let report = json!({
        "schema_version": 1,
        "scope": "isolated_authenticated_local_service_dispatch_not_native_ui_acceptance",
        "client_sha256": codol_protocol::CLIENT_SHA256,
        "game_endpoint_enabled": true,
        "synthetic_distinct_identities": true,
        "live_backend_or_account_data_modified": false,
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
