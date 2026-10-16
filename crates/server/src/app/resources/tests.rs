use super::*;
use serde_json::{Value, json};
use std::{cell::Cell, path::Path};

fn source(root: &Path) {
    std::fs::create_dir(root).unwrap();
    crate::game::session::catalog::write_synthetic(&root.join("war.json"), "mp_nuked");
    let mut state: Value =
        serde_json::from_slice(&std::fs::read(root.join("war.json")).unwrap()).unwrap();
    state["collision_file"] = json!("collision.json");
    for mode in ["war", "cha"] {
        state["mode"] = json!(mode);
        std::fs::write(root.join(format!("{mode}.json")), state.to_string()).unwrap();
    }
    std::fs::write(root.join("collision.json"),json!({
        "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
        "map":"maps/mp/mp_nuked.d3dbsp","ncs_crc":0x12345678u32,
        "brushes":[{"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]
    }).to_string()).unwrap();
    std::fs::write(
        root.join("catalog.json"),
        json!({"schema_version":1,"entries":[
            {"map":"mp_nuked","mode":"war","file":"war.json"},
            {"map":"mp_nuked","mode":"cha","file":"cha.json"}
        ]})
        .to_string(),
    )
    .unwrap();
}

#[test]
fn bundle_deduplicates_and_can_be_relocated_without_any_source_files() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    source(&input);
    let output = temp.path().join("output");
    let result = prepare(&input, &output, |_| Ok(()), || false).unwrap();
    assert_eq!(result["maps"], 2);
    assert_eq!(result["asset_files"], 1);
    let state: Value =
        serde_json::from_slice(&std::fs::read(output.join("maps/mp_nuked-war.json")).unwrap())
            .unwrap();
    assert!(
        state["collision_file"]
            .as_str()
            .unwrap()
            .starts_with("../assets/")
    );
    std::fs::remove_dir_all(input).unwrap();
    let relocated = temp.path().join("relocated");
    std::fs::rename(output, &relocated).unwrap();
    crate::game::session::catalog::GameMapCatalog::load(&relocated.join("catalog.json")).unwrap();
    prepare(
        &relocated,
        &temp.path().join("second"),
        |_| Ok(()),
        || false,
    )
    .unwrap();
    let asset = std::fs::read_dir(relocated.join("assets"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(asset, b"{}").unwrap();
    let error = prepare(
        &relocated,
        &temp.path().join("corrupt"),
        |_| Ok(()),
        || false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("校验和"));
    assert!(!temp.path().join("corrupt").exists());
}

#[test]
fn failed_or_cancelled_bundles_leave_no_partial_output_and_never_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    source(&input);
    let output = temp.path().join("output");
    let cancelled = Cell::new(false);
    assert!(
        prepare(
            &input,
            &output,
            |_| {
                cancelled.set(true);
                Ok(())
            },
            || cancelled.get()
        )
        .is_err()
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    let collision = input.join("collision.json");
    let original = std::fs::read(&collision).unwrap();
    let mut wrong: Value = serde_json::from_slice(&original).unwrap();
    wrong["ncs_crc"] = json!(7);
    std::fs::write(&collision, wrong.to_string()).unwrap();
    assert!(prepare(&input, &output, |_| Ok(()), || false).is_err());
    std::fs::remove_file(&collision).unwrap();
    assert!(prepare(&input, &output, |_| Ok(()), || false).is_err());
    std::fs::write(collision, original).unwrap();
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("keep"), "existing").unwrap();
    assert!(prepare(&input, &output, |_| Ok(()), || false).is_err());
    assert_eq!(std::fs::read(output.join("keep")).unwrap(), b"existing");
}

#[test]
fn unknown_catalog_fields_and_pathless_requests_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    source(&input);
    let path = input.join("catalog.json");
    let mut data: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    data["fallback"] = json!("mp_afghan");
    std::fs::write(path, data.to_string()).unwrap();
    assert!(prepare(&input, &temp.path().join("output"), |_| Ok(()), || false).is_err());
    let request: Request =
        serde_json::from_value(json!({"source":"relative","output_parent":temp.path()})).unwrap();
    assert!(request.validate().is_err());
}
