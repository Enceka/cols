//! Legacy 128-byte AUTH ticket. Layout corroborated in 3.3.3.3 at VA 0xa678c0;
//! algorithms/default password also corroborated in its own login call chain.
//! See ALPHA_BUILD_REVIEW.md. These algorithms are for game compatibility only.
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::NoPadding};
use std::{error::Error, fmt};
use tiger::{Digest, Tiger};

pub const TICKET_SIZE: usize = 128;
pub const MAGIC: u32 = 0xefbdadde;
pub const HASH_MAGIC: [u8; 3] = [9, 4, 0];

pub struct Ticket {
    pub title_id: u32,
    pub issued_at: u32,
    pub expires_at: u32,
    pub license_id: u64,
    pub user_id: u64,
    pub username: String,
    pub session_key: [u8; 24],
}

#[derive(Debug)]
pub struct TicketError(pub &'static str);
impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for TicketError {}

pub fn tiger192(bytes: &[u8]) -> [u8; 24] {
    Tiger::digest(bytes).into()
}
pub fn initial_vector(seed: u32) -> [u8; 8] {
    let digest = tiger192(&seed.to_le_bytes());
    let mut iv = [0; 8];
    iv.copy_from_slice(&digest[..8]);
    iv
}
pub fn password_key() -> [u8; 24] {
    tiger192(b"simplePassword")
}

impl Ticket {
    pub fn encode(&self) -> Result<[u8; TICKET_SIZE], TicketError> {
        if self.username.len() > 63 || self.username.as_bytes().contains(&0) {
            return Err(TicketError(
                "ticket username must fit a NUL-terminated 64-byte field",
            ));
        }
        if self.expires_at <= self.issued_at {
            return Err(TicketError("ticket expiry must follow issue time"));
        }
        let mut bytes = [0; TICKET_SIZE];
        bytes[..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[4] = 0;
        bytes[5..9].copy_from_slice(&self.title_id.to_le_bytes());
        bytes[9..13].copy_from_slice(&self.issued_at.to_le_bytes());
        bytes[13..17].copy_from_slice(&self.expires_at.to_le_bytes());
        bytes[17..25].copy_from_slice(&self.license_id.to_le_bytes());
        bytes[25..33].copy_from_slice(&self.user_id.to_le_bytes());
        bytes[33..33 + self.username.len()].copy_from_slice(self.username.as_bytes());
        bytes[97..121].copy_from_slice(&self.session_key);
        bytes[121..124].copy_from_slice(&HASH_MAGIC);
        let hash = tiger192(&bytes[..121]);
        bytes[124..].copy_from_slice(&hash[..4]);
        Ok(bytes)
    }
}

pub fn encrypt_ticket(
    plain: &[u8; TICKET_SIZE],
    iv_seed: u32,
) -> Result<[u8; TICKET_SIZE], TicketError> {
    let mut output = *plain;
    cbc::Encryptor::<des::TdesEde3>::new(&password_key().into(), &initial_vector(iv_seed).into())
        .encrypt_padded_mut::<NoPadding>(&mut output, TICKET_SIZE)
        .map_err(|_| TicketError("ticket encryption failed"))?;
    Ok(output)
}

pub fn decrypt_ticket(
    cipher: &[u8; TICKET_SIZE],
    iv_seed: u32,
) -> Result<[u8; TICKET_SIZE], TicketError> {
    let mut output = *cipher;
    cbc::Decryptor::<des::TdesEde3>::new(&password_key().into(), &initial_vector(iv_seed).into())
        .decrypt_padded_mut::<NoPadding>(&mut output)
        .map_err(|_| TicketError("ticket decryption failed"))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    #[test]
    fn matches_independent_reference_layout_digest_iv_and_ciphertext() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/auth-ticket-alpha-vector.json"
        )))
        .unwrap();
        let key = std::array::from_fn(|i| i as u8);
        let ticket = Ticket {
            title_id: 1002,
            issued_at: 0,
            expires_at: 0x7fffffff,
            license_id: 1002,
            user_id: 1,
            username: "alpha0022".into(),
            session_key: key,
        };
        let plain = ticket.encode().unwrap();
        let seed = 0x12345678;
        assert_eq!(hex(&plain), vector["plaintext_hex"]);
        assert_eq!(hex(&password_key()), vector["key_hex"]);
        assert_eq!(hex(&initial_vector(seed)), vector["iv_hex"]);
        let cipher = encrypt_ticket(&plain, seed).unwrap();
        assert_eq!(hex(&cipher), vector["ciphertext_hex"]);
        assert_eq!(decrypt_ticket(&cipher, seed).unwrap(), plain);
    }
    #[test]
    fn preserves_unsigned_ids_and_rejects_invalid_field_boundaries() {
        let mut ticket = Ticket {
            title_id: u32::MAX,
            issued_at: 1,
            expires_at: 2,
            license_id: u64::MAX,
            user_id: u64::MAX,
            username: "a".repeat(63),
            session_key: [0x42; 24],
        };
        let bytes = ticket.encode().unwrap();
        assert_eq!(&bytes[17..33], &[0xff; 16]);
        assert_eq!(bytes[96], 0);
        ticket.username.push('b');
        assert!(ticket.encode().is_err());
        ticket.username = "a\0b".into();
        assert!(ticket.encode().is_err());
        ticket.username = "player".into();
        ticket.expires_at = 1;
        assert!(ticket.encode().is_err());
    }
}
