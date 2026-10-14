//! Process-local readiness contract for the standalone native launcher.
use crate::infra::storage::atomic_private_write;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::path::Path;

pub(super) async fn publish(path: &Path, mut payload: Value) -> Result<()> {
    ensure!(
        matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "launcher readiness destination must not exist"
    );
    let object = payload
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("readiness payload must be an object"))?;
    object.insert("schema_version".into(), json!(1));
    object.insert("pid".into(), json!(std::process::id()));
    object.insert("client_sha256".into(), json!(codol_protocol::CLIENT_SHA256));
    atomic_private_write(path, &serde_json::to_vec_pretty(&payload)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn private_ready_record_has_identity_and_never_replaces_existing_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ready.json");
        publish(&path, json!({"kind":"tcp_observer","listeners":{}}))
            .await
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["pid"], std::process::id());
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["client_sha256"], codol_protocol::CLIENT_SHA256);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(
            publish(&path, json!({"kind":"native_services"}))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
