//! Prepare a local player's template for the native dw_username login.
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let name = std::env::args()
        .nth(1)
        .context("usage: native_identity USERNAME")?;
    let id =
        codol_protocol::auth::local_identity::user_id(&name).context("invalid local username")?;
    println!(
        "{}",
        serde_json::json!({"username":name,"user_id":id.to_string()})
    );
    Ok(())
}
