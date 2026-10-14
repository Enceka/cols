//! CODOL 3.3.3.3 Demonware v2 game DATA records.
//! Target KDF: 0xa7af90; DATA: 0xa7fda0; replay window: 0xb2da20.
//! Unlike AlphaBuild 1.1.1.13, this target uses SHA1/AES128 and hashes the tag
//! into the IV. See docs/GAME_TRANSPORT_PROGRESS.md for runtime evidence.
use crate::lobby::channel::LobbyError;
use aes::cipher::{BlockEncrypt, KeyInit};
use hkdf::hmac::{Hmac, Mac};
use sha1::{Digest, Sha1};

pub fn derive_key(shared_x: &[u8; 28], security: &[u8; 16]) -> [u8; 24] {
    let mut input = [0; 44];
    input[..28].copy_from_slice(shared_x);
    input[28..].copy_from_slice(security);
    let mut key = [0; 24];
    key[..20].copy_from_slice(&Sha1::digest(input));
    key
}
fn crypt(data: &mut [u8], counter: u32, tag: u16, key: &[u8; 24]) {
    let mut seed = [0; 6];
    seed[..4].copy_from_slice(&counter.to_le_bytes());
    seed[4..].copy_from_slice(&tag.to_le_bytes());
    let hash = Sha1::digest(seed);
    let mut nonce = u128::from_le_bytes(hash[..16].try_into().expect("SHA1 prefix"));
    let aes = aes::Aes128::new_from_slice(&key[..16]).expect("AES128 key width");
    for chunk in data.chunks_mut(16) {
        let mut block = aes::Block::from(nonce.to_le_bytes());
        aes.encrypt_block(&mut block);
        for (byte, mask) in chunk.iter_mut().zip(block) {
            *byte ^= mask;
        }
        nonce = nonce.wrapping_add(1);
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Data {
    pub encrypted: Vec<u8>,
    pub cleartext: Vec<u8>,
}

pub struct Records {
    key: [u8; 24],
    receive_high: Option<u32>,
    seen: u32,
    send_next: Option<u32>,
}
impl Records {
    pub fn new(key: [u8; 24]) -> Self {
        Self {
            key,
            receive_high: None,
            seen: 0,
            send_next: Some(1),
        }
    }
    pub fn open(&mut self, wire: &[u8], tag: u16) -> Result<Data, LobbyError> {
        if wire.len() < 16
            || wire[..2] != [6, 2]
            || u16::from_le_bytes([wire[2], wire[3]]) != tag
            || usize::from(u16::from_le_bytes([wire[14], wire[15]])) > wire.len() - 16
        {
            return Err(LobbyError("invalid game DATA record"));
        }
        let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(&self.key)
            .map_err(|_| LobbyError("game MAC key"))?;
        mac.update(&wire[..6]);
        mac.update(&wire[14..]);
        mac.verify_truncated_left(&wire[6..14])
            .map_err(|_| LobbyError("game DATA MAC mismatch"))?;
        let low = u32::from(u16::from_le_bytes([wire[4], wire[5]]));
        let mut counter = low;
        let seen = if let Some(high) = self.receive_high {
            counter = (high & 0xffff0000) | low;
            if counter > high && counter - high > 32768 {
                counter = counter
                    .checked_sub(65536)
                    .ok_or(LobbyError("game counter underflow"))?;
            } else if high > counter && high - counter > 32768 {
                counter = counter
                    .checked_add(65536)
                    .ok_or(LobbyError("game counter overflow"))?;
            }
            if counter <= high {
                let distance = high - counter;
                if distance >= 32 || self.seen & (1u32 << distance) != 0 {
                    return Err(LobbyError("game DATA replay/window violation"));
                }
                self.seen | (1u32 << distance)
            } else {
                self.seen.checked_shl(counter - high).unwrap_or(0) | 1
            }
        } else {
            1
        };
        let end = 16 + usize::from(u16::from_le_bytes([wire[14], wire[15]]));
        let mut encrypted = wire[16..end].to_vec();
        crypt(&mut encrypted, counter, tag, &self.key);
        self.receive_high = Some(self.receive_high.map_or(counter, |v| v.max(counter)));
        self.seen = seen;
        Ok(Data {
            encrypted,
            cleartext: wire[end..].to_vec(),
        })
    }
    pub fn seal(&mut self, plain: &[u8], tag: u16) -> Result<Vec<u8>, LobbyError> {
        self.seal_split(plain, &[], tag)
    }
    pub fn seal_split(
        &mut self,
        plain: &[u8],
        cleartext: &[u8],
        tag: u16,
    ) -> Result<Vec<u8>, LobbyError> {
        if plain.len() > u16::MAX as usize
            || plain.len().saturating_add(cleartext.len()) > u16::MAX as usize - 16
        {
            return Err(LobbyError("game DATA length limit"));
        }
        let counter = self
            .send_next
            .ok_or(LobbyError("game send counter exhausted"))?;
        let mut wire = vec![0; 16];
        wire[..2].copy_from_slice(&[6, 2]);
        wire[2..4].copy_from_slice(&tag.to_le_bytes());
        wire[4..6].copy_from_slice(&(counter as u16).to_le_bytes());
        wire[14..16].copy_from_slice(&(plain.len() as u16).to_le_bytes());
        wire.extend_from_slice(plain);
        crypt(&mut wire[16..], counter, tag, &self.key);
        wire.extend_from_slice(cleartext);
        let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(&self.key)
            .map_err(|_| LobbyError("game MAC key"))?;
        mac.update(&wire[..6]);
        mac.update(&wire[14..]);
        wire[6..14].copy_from_slice(&mac.finalize().into_bytes()[..8]);
        self.send_next = counter.checked_add(1);
        Ok(wire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_sha1_aes128_vector_with_authenticated_clear_tail() {
        // Generated independently with Python hashlib/hmac + PyCryptodome AES.
        // Synthetic inputs only; no client credentials or captured session keys.
        let key = derive_key(
            &std::array::from_fn(|i| i as u8),
            &std::array::from_fn(|i| 0xa0 + i as u8),
        );
        let hex = "060234127856a89558242daf6c8f1600793788c6326f2271e41170fa029e9b37891189a716da636c6561722d7461696c";
        let expected = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let mut sender = Records::new(key);
        sender.send_next = Some(0x12345678);
        assert_eq!(
            sender
                .seal_split(
                    b"\xff\xff\xff\xffgetchallenge 1234\n",
                    b"clear-tail",
                    0x1234
                )
                .unwrap(),
            expected
        );
        let mut receiver = Records::new(key);
        receiver.receive_high = Some(0x12345677);
        for at in [4, 6, 14, 16, expected.len() - 1] {
            let mut damaged = expected.clone();
            damaged[at] ^= 1;
            assert!(receiver.open(&damaged, 0x1234).is_err());
        }
        let actual = receiver.open(&expected, 0x1234).unwrap();
        assert_eq!(actual.encrypted, b"\xff\xff\xff\xffgetchallenge 1234\n");
        assert_eq!(actual.cleartext, b"clear-tail");
    }

    #[test]
    fn replay_window_is_32_records_and_send_counter_cannot_repeat() {
        let key = [7; 24];
        let mut sender = Records::new(key);
        let mut receiver = Records::new(key);
        let packets = (0..33)
            .map(|_| sender.seal(b"test", 42).unwrap())
            .collect::<Vec<_>>();
        receiver.open(&packets[32], 42).unwrap();
        receiver.open(&packets[1], 42).unwrap();
        assert!(receiver.open(&packets[0], 42).is_err());
        assert!(receiver.open(&packets[1], 42).is_err());
        sender.send_next = Some(u32::MAX);
        sender.seal(b"last", 42).unwrap();
        assert!(sender.seal(b"overflow", 42).is_err());
    }
    #[test]
    fn rejects_corruption_without_consuming_counter_and_accepts_reordered_packets() {
        let key = [19; 24];
        let mut sender = Records::new(key);
        let mut receiver = Records::new(key);
        let a = sender.seal(b"first", 42).unwrap();
        let b = sender.seal(b"second", 42).unwrap();
        for at in [0, 1, 2, 4, 6, 14, 16] {
            let mut damaged = a.clone();
            damaged[at] ^= 1;
            assert!(receiver.open(&damaged, 42).is_err());
        }
        assert_eq!(receiver.open(&b, 42).unwrap().encrypted, b"second");
        assert_eq!(receiver.open(&a, 42).unwrap().encrypted, b"first");
        assert!(receiver.open(&a, 42).is_err());
        sender.send_next = Some(65535);
        let c = sender.seal(b"near wrap", 42).unwrap();
        let mut receiver = Records::new(key);
        assert_eq!(receiver.open(&c, 42).unwrap().encrypted, b"near wrap");
        let d = sender.seal(b"wrapped", 42).unwrap();
        assert_eq!(receiver.open(&d, 42).unwrap().encrypted, b"wrapped");
    }
}
