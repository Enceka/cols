//! The observed JSON body of HTTPS POST /auth/ task 10.
//! This module parses requests only. Ticket generation and responses are not implemented.

use crate::ticket::{Ticket, TicketError, encrypt_ticket};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, error::Error, fmt};

// Research input limit, not a claim about every supported server-side payload.
pub const MAX_AUTH_BODY: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
pub struct Task10Wire {
    pub auth_task: String,
    pub iv_seed: String,
    pub title_id: String,
    pub identity: String,
    /// Deliberately a JSON-encoded STRING, not an embedded object.
    pub extra_data: String,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct Task10Extra {
    pub version: String,
    pub user_id: String,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug)]
pub struct Task10Request {
    pub wire: Task10Wire,
    pub extra: Task10Extra,
    pub iv_seed: u32,
    pub title_id: u32,
    pub user_id: u64,
}

/// Response framing follows the 3.3.3.3 JSON reader. The ticket body follows
/// the corroborated legacy layout; real-client acceptance is tracked separately.
#[derive(Serialize)]
pub struct Task10Response {
    pub auth_task: &'static str,
    pub code: &'static str,
    pub iv_seed: String,
    pub client_ticket: String,
    pub server_ticket: String,
}

pub fn local_ticket_response(
    request: &Task10Request,
    ticket: &Ticket,
) -> Result<Task10Response, TicketError> {
    if ticket.title_id != request.title_id || ticket.user_id != request.user_id {
        return Err(TicketError("ticket does not belong to this AUTH request"));
    }
    let bytes = encrypt_ticket(&ticket.encode()?, request.iv_seed)?;
    let encoded = STANDARD.encode(bytes);
    Ok(Task10Response {
        auth_task: "11",
        code: "700",
        iv_seed: request.iv_seed.to_string(),
        client_ticket: encoded.clone(),
        server_ticket: encoded,
    })
}

#[derive(Debug)]
pub enum AuthError {
    TooLarge,
    Json(serde_json::Error),
    UnsupportedTask,
    InvalidDecimal(&'static str),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("AUTH JSON exceeds research input limit"),
            Self::Json(error) => write!(f, "AUTH JSON: {error}"),
            Self::UnsupportedTask => f.write_str("only the observed auth_task 10 is parsed"),
            Self::InvalidDecimal(field) => write!(f, "invalid unsigned decimal string: {field}"),
        }
    }
}

impl Error for AuthError {}

fn decimal<T: std::str::FromStr>(value: &str, field: &'static str) -> Result<T, AuthError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AuthError::InvalidDecimal(field));
    }
    value.parse().map_err(|_| AuthError::InvalidDecimal(field))
}

pub fn parse_task10(body: &[u8]) -> Result<Task10Request, AuthError> {
    if body.len() > MAX_AUTH_BODY {
        return Err(AuthError::TooLarge);
    }
    let wire: Task10Wire = serde_json::from_slice(body).map_err(AuthError::Json)?;
    if wire.auth_task != "10" {
        return Err(AuthError::UnsupportedTask);
    }
    let extra: Task10Extra = serde_json::from_str(&wire.extra_data).map_err(AuthError::Json)?;
    Ok(Task10Request {
        iv_seed: decimal(&wire.iv_seed, "iv_seed")?,
        title_id: decimal(&wire.title_id, "title_id")?,
        user_id: decimal(&extra.user_id, "user_id")?,
        wire,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const SANITIZED: &[u8] =
        include_bytes!("../../../fixtures/protocol/auth-task10-sanitized.json");

    #[test]
    fn reads_observed_string_types_and_nested_json_string() {
        let request = parse_task10(SANITIZED).unwrap();
        assert_eq!(request.title_id, 1002);
        assert_eq!(request.iv_seed, 1);
        assert_eq!(request.user_id, 2);
        assert_eq!(request.wire.identity, "356c4bc3");
        assert_eq!(request.extra.version, "0");
    }

    #[test]
    fn preserves_user_ids_above_signed_64_bit_range() {
        let mut body: serde_json::Value = serde_json::from_slice(SANITIZED).unwrap();
        body["extra_data"] =
            serde_json::json!("{\"version\":\"0\",\"user_id\":\"18446744073709551615\"}");
        let request = parse_task10(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert_eq!(request.user_id, u64::MAX);
    }

    #[test]
    fn rejects_wrong_wire_types_and_overflow_without_guessing_other_tasks() {
        let original: serde_json::Value = serde_json::from_slice(SANITIZED).unwrap();
        for (field, value) in [
            ("auth_task", serde_json::json!(10)),
            ("auth_task", serde_json::json!("11")),
            ("title_id", serde_json::json!("4294967296")),
            ("iv_seed", serde_json::json!("-1")),
            (
                "extra_data",
                serde_json::json!({"version":"0", "user_id":"2"}),
            ),
            (
                "extra_data",
                serde_json::json!("{\"version\":\"0\",\"user_id\":\"18446744073709551616\"}"),
            ),
        ] {
            let mut malformed = original.clone();
            malformed[field] = value;
            assert!(
                parse_task10(&serde_json::to_vec(&malformed).unwrap()).is_err(),
                "{field}"
            );
        }
        assert!(matches!(
            parse_task10(&vec![b' '; MAX_AUTH_BODY + 1]),
            Err(AuthError::TooLarge)
        ));
    }
}
