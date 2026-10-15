//! Native local login: 0x6d2f10 Base64-encodes dw_username; 0xa52620
//! lowercases that identity, hashes it with Tiger192 and reads its first u64.
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub fn user_id(username: &str) -> Option<u64> {
    if username.is_empty()
        || username.len() > 15
        || !username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return None;
    }
    let identity = STANDARD.encode(username).to_ascii_lowercase();
    let digest = super::ticket::tiger192(identity.as_bytes());
    Some(u64::from_le_bytes(digest[..8].try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_distinct_native_identities_and_bounded_ascii_input() {
        assert_eq!(super::user_id("Player_A"), super::user_id("Player_A"));
        assert_ne!(super::user_id("Player_A"), super::user_id("Player_B"));
        for invalid in ["", "too_long_identity", "a;b", "玩家"] {
            assert!(super::user_id(invalid).is_none());
        }
    }
}
