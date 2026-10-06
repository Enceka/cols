//! configs_s.info encryption, recovered from the unmodified target executable.
//! AES-192-CTR, counter LE32(byte length) || 01*12, incremented big-endian.
//! This static resource key is not an account/session secret. Recovery evidence
//! and independent known-answer vectors are in docs/PUBLISHER_FILES.md.
use crate::{lobby::LobbyError, task::MAX_FILE_BYTES};
use aes::cipher::{BlockEncrypt, KeyInit};

const RESOURCE_KEY: [u8; 24] = [
    0x15, 0x9a, 0x03, 0x25, 0xe0, 0x75, 0x2e, 0x80, 0xc6, 0xc0, 0x94, 0x2a, 0x50, 0x5c, 0x1c, 0x68,
    0x8c, 0x17, 0xef, 0x53, 0x99, 0xf8, 0x68, 0x3c,
];

/// Length-preserving and symmetric. No extra NUL, padding, or IV on the wire.
pub fn crypt_configs(data: &[u8]) -> Result<Vec<u8>, LobbyError> {
    if data.is_empty() || data.len() > MAX_FILE_BYTES {
        return Err(LobbyError("configs_s size exceeds local research range"));
    }
    let cipher = aes::Aes192::new((&RESOURCE_KEY).into());
    let mut iv = [1u8; 16];
    iv[..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
    let mut counter = u128::from_be_bytes(iv);
    let mut out = data.to_vec();
    for chunk in out.chunks_mut(16) {
        let mut block = aes::Block::from(counter.to_be_bytes());
        cipher.encrypt_block(&mut block);
        for (byte, key) in chunk.iter_mut().zip(block) {
            *byte ^= key;
        }
        counter = counter.wrapping_add(1);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agrees_with_independent_aes_ctr_vectors_and_length_derived_iv() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/publisher3333.json"
        ))
        .unwrap();
        let unhex = |s: &str| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>()
        };
        for vector in vectors["vectors"].as_array().unwrap() {
            let plain = unhex(vector["plaintext"].as_str().unwrap());
            let encrypted = unhex(vector["ciphertext"].as_str().unwrap());
            assert_eq!(crypt_configs(&plain).unwrap(), encrypted);
            assert_eq!(crypt_configs(&encrypted).unwrap(), plain);
        }
        assert!(crypt_configs(&[]).is_err());
        assert!(crypt_configs(&vec![0; MAX_FILE_BYTES + 1]).is_err());
    }
}
