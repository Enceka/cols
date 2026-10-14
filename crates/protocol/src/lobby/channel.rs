//! Protocol-210 negotiation, recovered at VA 0xa610c0/0xa61980/0xa618c0.
//! Authenticated application framing; service dispatch lives in the server crate.
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::NoPadding};
use hkdf::Hkdf;
use hkdf::hmac::{Hmac, Mac};
use sha1::{Digest, Sha1};
use std::{error::Error, fmt};

/// The shipped protocol-210 hello advertises a 0x80000-byte receive frame.
/// Reserve space for the application header, AES padding and frame MAC.
pub const MAX_SERVER_FRAME_BYTES: usize = 512 * 1024;
pub const MAX_SERVER_APPLICATION_BODY_BYTES: usize = MAX_SERVER_FRAME_BYTES - 64;

#[derive(Debug)]
pub struct LobbyError(pub &'static str);
impl fmt::Display for LobbyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for LobbyError {}

pub struct ClientHello {
    pub max_receive: u32,
    pub nonce: [u8; 8],
}
pub fn read_client_hello(bytes: &[u8; 28]) -> Result<ClientHello, LobbyError> {
    if bytes[..16]
        != [200u32, 200, 210, 210]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect::<Vec<_>>()
    {
        return Err(LobbyError("unsupported lobby protocol preface"));
    }
    let max_receive = u32::from_le_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| LobbyError("preface size"))?,
    );
    if !(26..=16 * 1024 * 1024).contains(&max_receive) {
        return Err(LobbyError("invalid lobby receive limit"));
    }
    let nonce = bytes[20..28]
        .try_into()
        .map_err(|_| LobbyError("nonce size"))?;
    Ok(ClientHello { max_receive, nonce })
}

pub fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut wire = Vec::with_capacity(body.len() + 6);
    wire.extend_from_slice(&((body.len() + 2) as u32).to_le_bytes());
    wire.extend_from_slice(&[0xab, kind]);
    wire.extend_from_slice(body);
    wire
}
pub fn server_hello(connection_id: u64, nonce: [u8; 8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&210u32.to_le_bytes());
    payload.extend_from_slice(&connection_id.to_le_bytes());
    payload.extend_from_slice(&nonce);
    frame(0x81, &payload)
}

pub struct ClientAuth {
    pub title_id: u32,
    pub iv_seed: u32,
    pub server_ticket: [u8; 128],
    pub proof: [u8; 8],
}

struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u32, LobbyError> {
        if n > 32 || n > self.bytes.len() * 8 - self.position {
            return Err(LobbyError("truncated lobby bit buffer"));
        }
        let mut value = 0;
        for i in 0..n {
            value |= u32::from((self.bytes[self.position / 8] >> (self.position % 8)) & 1) << i;
            self.position += 1;
        }
        Ok(value)
    }
}
pub fn parse_client_auth(wire: &[u8]) -> Result<ClientAuth, LobbyError> {
    if wire.len() < 14 || wire[4..6] != [0xab, 0x82] {
        return Err(LobbyError("expected AB82 client auth"));
    }
    let length = u32::from_le_bytes(
        wire[..4]
            .try_into()
            .map_err(|_| LobbyError("auth length"))?,
    ) as usize;
    if length != wire.len() - 4 {
        return Err(LobbyError("inconsistent auth frame length"));
    }
    let mut bits = Bits {
        bytes: &wire[6..wire.len() - 8],
        position: 0,
    };
    if bits.read(8)? != 7 || bits.read(1)? != 1 || bits.read(5)? != 8 {
        return Err(LobbyError("unexpected auth bit-buffer prefix"));
    }
    let title_id = bits.read(32)?;
    if bits.read(5)? != 8 {
        return Err(LobbyError("expected IV-seed tag"));
    }
    let iv_seed = bits.read(32)?;
    let mut server_ticket = [0; 128];
    for byte in &mut server_ticket {
        *byte = bits.read(8)? as u8;
    }
    let remaining = bits.bytes.len() * 8 - bits.position;
    if remaining > 7 || bits.read(remaining)? != 0 {
        return Err(LobbyError("unexpected data after auth ticket"));
    }
    let proof = wire[wire.len() - 8..]
        .try_into()
        .map_err(|_| LobbyError("auth proof length"))?;
    Ok(ClientAuth {
        title_id,
        iv_seed,
        server_ticket,
        proof,
    })
}

pub struct HandshakeSecrets {
    pub client_proof: [u8; 8],
    pub server_proof: [u8; 8],
    /// 20+20 byte MAC keys, then 16+16 byte cipher keys; direction needs separate evidence.
    pub data_keys: [u8; 72],
}

#[derive(Clone, Copy)]
pub enum Direction {
    ClientToServer,
    ServerToClient,
}
pub struct ApplicationMessage {
    pub sequence: u32,
    pub kind: u8,
    pub body: Vec<u8>,
}

fn keys(data: &[u8; 72], direction: Direction) -> (&[u8], &[u8]) {
    match direction {
        Direction::ClientToServer => (&data[..20], &data[40..56]),
        Direction::ServerToClient => (&data[20..40], &data[56..72]),
    }
}

pub fn decrypt_application(
    wire: &[u8],
    data: &[u8; 72],
    direction: Direction,
    expected_sequence: u32,
) -> Result<ApplicationMessage, LobbyError> {
    let frame_limit = match direction {
        Direction::ClientToServer => 65536,
        Direction::ServerToClient => MAX_SERVER_FRAME_BYTES,
    };
    if wire.len() < 50 || wire.len() > frame_limit + 4 || wire[4..6] != [0xab, 0x85] {
        return Err(LobbyError("invalid encrypted lobby frame"));
    }
    let length =
        u32::from_le_bytes(wire[..4].try_into().map_err(|_| LobbyError("frame size"))?) as usize;
    if length != wire.len() - 4 || !(wire.len() - 34).is_multiple_of(16) {
        return Err(LobbyError("invalid lobby cipher length"));
    }
    let sequence = u32::from_le_bytes(
        wire[6..10]
            .try_into()
            .map_err(|_| LobbyError("sequence size"))?,
    );
    if sequence != expected_sequence {
        return Err(LobbyError("out-of-order lobby sequence"));
    }
    let (mac_key, cipher_key) = keys(data, direction);
    let mut mac =
        <Hmac<Sha1> as Mac>::new_from_slice(mac_key).map_err(|_| LobbyError("MAC key"))?;
    mac.update(&wire[..wire.len() - 8]);
    mac.verify_truncated_left(&wire[wire.len() - 8..])
        .map_err(|_| LobbyError("lobby MAC mismatch"))?;
    let mut plain = wire[26..wire.len() - 8].to_vec();
    cbc::Decryptor::<aes::Aes128>::new_from_slices(cipher_key, &wire[10..26])
        .map_err(|_| LobbyError("cipher key/IV"))?
        .decrypt_padded_mut::<NoPadding>(&mut plain)
        .map_err(|_| LobbyError("lobby decryption"))?;
    let size = u32::from_le_bytes(
        plain[..4]
            .try_into()
            .map_err(|_| LobbyError("plaintext size"))?,
    ) as usize;
    if size > plain.len() - 5 || plain.len() - 5 - size >= 16 {
        return Err(LobbyError("invalid lobby plaintext length"));
    }
    Ok(ApplicationMessage {
        sequence,
        kind: plain[4],
        body: plain[5..5 + size].to_vec(),
    })
}

pub fn encrypt_application(
    kind: u8,
    body: &[u8],
    data: &[u8; 72],
    direction: Direction,
    sequence: u32,
    iv: [u8; 16],
) -> Result<Vec<u8>, LobbyError> {
    let body_limit = match direction {
        Direction::ClientToServer => 65000,
        Direction::ServerToClient => MAX_SERVER_APPLICATION_BODY_BYTES,
    };
    if body.len() > body_limit {
        return Err(LobbyError("application body limit"));
    }
    let size = (body.len() + 5).div_ceil(16) * 16;
    let mut plain = vec![0; size];
    plain[..4].copy_from_slice(&(body.len() as u32).to_le_bytes());
    plain[4] = kind;
    plain[5..5 + body.len()].copy_from_slice(body);
    let (mac_key, cipher_key) = keys(data, direction);
    cbc::Encryptor::<aes::Aes128>::new_from_slices(cipher_key, &iv)
        .map_err(|_| LobbyError("cipher key/IV"))?
        .encrypt_padded_mut::<NoPadding>(&mut plain, size)
        .map_err(|_| LobbyError("lobby encryption"))?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&sequence.to_le_bytes());
    payload.extend_from_slice(&iv);
    payload.extend_from_slice(&plain);
    payload.extend_from_slice(&[0; 8]);
    let mut wire = frame(0x85, &payload);
    let end = wire.len() - 8;
    let mut mac =
        <Hmac<Sha1> as Mac>::new_from_slice(mac_key).map_err(|_| LobbyError("MAC key"))?;
    mac.update(&wire[..end]);
    wire[end..].copy_from_slice(&mac.finalize().into_bytes()[..8]);
    Ok(wire)
}

/// Task 71/4 serializer corroborated at VA 0xa48800: two Blob16s, a UInt32,
/// two optional blobs and a bounded string. No result array is bound there.
pub fn registration_build(body: &[u8]) -> Result<u32, LobbyError> {
    if !body.starts_with(&[71, 3, 4]) {
        return Err(LobbyError("not a registration task"));
    }
    let mut pos = 3;
    fn blob(bytes: &[u8], pos: &mut usize) -> Result<usize, LobbyError> {
        if bytes.get(*pos..*pos + 2) != Some(&[19, 8][..]) {
            return Err(LobbyError("expected tagged blob"));
        }
        *pos += 2;
        let raw = bytes
            .get(*pos..*pos + 4)
            .ok_or(LobbyError("truncated blob length"))?;
        let count =
            u32::from_le_bytes(raw.try_into().map_err(|_| LobbyError("blob length"))?) as usize;
        *pos += 4;
        if count > bytes.len() - *pos {
            return Err(LobbyError("truncated blob"));
        }
        *pos += count;
        Ok(count)
    }
    if blob(body, &mut pos)? != 16 || blob(body, &mut pos)? != 16 {
        return Err(LobbyError("registration identifiers must be 16 bytes"));
    }
    if body.get(pos) != Some(&8) {
        return Err(LobbyError("expected build UInt32"));
    }
    pos += 1;
    let raw = body
        .get(pos..pos + 4)
        .ok_or(LobbyError("truncated build"))?;
    let build = u32::from_le_bytes(raw.try_into().map_err(|_| LobbyError("build length"))?);
    pos += 4;
    blob(body, &mut pos)?;
    blob(body, &mut pos)?;
    if body.get(pos) != Some(&16) {
        return Err(LobbyError("expected registration string"));
    }
    pos += 1;
    let end = body[pos..]
        .iter()
        .position(|b| *b == 0)
        .ok_or(LobbyError("unterminated registration string"))?;
    if end > 18 {
        return Err(LobbyError("registration string exceeds limit"));
    }
    pos += end + 1;
    if body[pos..].len() > 1 || body[pos..].iter().any(|b| *b != 0) {
        return Err(LobbyError("unexpected registration tail"));
    }
    Ok(build)
}

pub fn registration_status(transaction_id: u64) -> Vec<u8> {
    let mut body = vec![10];
    body.extend_from_slice(&transaction_id.to_le_bytes());
    body.push(8);
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&[3, 4, 8]);
    body.extend_from_slice(&0u32.to_le_bytes());
    body
}
pub fn derive_secrets(
    client: &[u8; 28],
    server: &[u8],
    auth: &[u8],
    key: &[u8; 24],
) -> Result<HandshakeSecrets, LobbyError> {
    read_client_hello(client)?;
    parse_client_auth(auth)?;
    if server.len() != 26 || server[4..6] != [0xab, 0x81] {
        return Err(LobbyError("invalid server hello transcript"));
    }
    let mut transcript = Vec::new();
    // The client hashes the negotiated 210 pair, not the initial 200 pair.
    transcript.extend_from_slice(&client[8..]);
    transcript.extend_from_slice(server);
    // Prefix length includes proof, but proof itself is excluded from the hash.
    transcript.extend_from_slice(&auth[..auth.len() - 8]);
    let hash = Sha1::digest(&transcript);
    let hkdf = Hkdf::<Sha1>::new(Some(&hash), key);
    let mut challenge = [0; 16];
    hkdf.expand(b"CLIENTCHAL", &mut challenge)
        .map_err(|_| LobbyError("challenge KDF"))?;
    let mut data_keys = [0; 72];
    hkdf.expand(b"BDDATA", &mut data_keys)
        .map_err(|_| LobbyError("data KDF"))?;
    Ok(HandshakeSecrets {
        client_proof: challenge[..8]
            .try_into()
            .map_err(|_| LobbyError("proof size"))?,
        server_proof: challenge[8..]
            .try_into()
            .map_err(|_| LobbyError("proof size"))?,
        data_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unhex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn agrees_with_independent_hmac_vector() {
        let value: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/lobby210-synthetic.json"
        )))
        .unwrap();
        let bytes = |name: &str| unhex(value[name].as_str().unwrap());
        let hello: [u8; 28] = bytes("client_hello").try_into().unwrap();
        let key: [u8; 24] = bytes("key").try_into().unwrap();
        let auth = bytes("client_auth");
        let parsed = parse_client_auth(&auth).unwrap();
        assert_eq!(parsed.title_id, 1002);
        assert_eq!(parsed.iv_seed, 0x11223344);
        assert_eq!(parsed.server_ticket, std::array::from_fn(|i| i as u8));
        let derived = derive_secrets(&hello, &bytes("server_hello"), &auth, &key).unwrap();
        assert_eq!(derived.client_proof.as_slice(), bytes("client_proof"));
        assert_eq!(derived.server_proof.as_slice(), bytes("server_proof"));
        assert_eq!(derived.data_keys.as_slice(), bytes("data_keys"));
    }
    #[test]
    fn malformed_bit_buffers_are_rejected_without_panics() {
        let value: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/lobby210-synthetic.json"
        )))
        .unwrap();
        let auth = unhex(value["client_auth"].as_str().unwrap());
        for n in 0..auth.len() {
            assert!(parse_client_auth(&auth[..n]).is_err());
        }
        for offset in [4, 5, 6, 7] {
            let mut bad = auth.clone();
            bad[offset] ^= 0xff;
            assert!(parse_client_auth(&bad).is_err());
        }
    }

    #[test]
    fn application_cipher_and_mac_match_independent_vector() {
        let value: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/lobby210-synthetic.json"
        )))
        .unwrap();
        let bytes = |key: &str| unhex(value[key].as_str().unwrap());
        let keys: [u8; 72] = bytes("data_keys").try_into().unwrap();
        let iv: [u8; 16] = bytes("application_iv").try_into().unwrap();
        let frame = bytes("application_frame");
        let decoded = decrypt_application(&frame, &keys, Direction::ClientToServer, 1).unwrap();
        assert_eq!(decoded.kind, 0x86);
        assert_eq!(decoded.body, bytes("application_message"));
        assert_eq!(
            encrypt_application(0x86, &decoded.body, &keys, Direction::ClientToServer, 1, iv)
                .unwrap(),
            frame
        );
        assert!(decrypt_application(&frame, &keys, Direction::ClientToServer, 2).is_err());
        assert!(decrypt_application(&frame, &keys, Direction::ServerToClient, 1).is_err());
        for offset in [0, 4, 10, 27, frame.len() - 1] {
            let mut bad = frame.clone();
            bad[offset] ^= 1;
            assert!(decrypt_application(&bad, &keys, Direction::ClientToServer, 1).is_err());
        }
    }

    #[test]
    fn registration_requires_the_observed_argument_shape() {
        let mut body = vec![71, 3, 4];
        for value in [0x11, 0x22] {
            body.extend_from_slice(&[19, 8, 16, 0, 0, 0]);
            body.extend_from_slice(&[value; 16]);
        }
        body.extend_from_slice(&[
            8, 0, 0, 0, 0, 19, 8, 0, 0, 0, 0, 19, 8, 0, 0, 0, 0, 16, 0, 0,
        ]);
        assert_eq!(registration_build(&body).unwrap(), 0);
        assert!(registration_build(&body[..20]).is_err());
        body[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(registration_build(&body).is_err());
        assert_eq!(
            registration_status(1),
            [
                10, 1, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 3, 4, 8, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn larger_server_frames_preserve_the_client_to_server_limit() {
        let hello = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/lobby-clienthello-3.3.3.3.bin"
        ));
        assert_eq!(
            read_client_hello(hello).unwrap().max_receive as usize,
            MAX_SERVER_FRAME_BYTES
        );
        let body = vec![0x3a; MAX_SERVER_APPLICATION_BODY_BYTES];
        let wire =
            encrypt_application(1, &body, &[1; 72], Direction::ServerToClient, 1, [2; 16]).unwrap();
        assert!(wire.len() - 4 <= MAX_SERVER_FRAME_BYTES);
        assert!(
            encrypt_application(1, &body, &[1; 72], Direction::ClientToServer, 1, [2; 16]).is_err()
        );
        assert!(decrypt_application(&wire, &[1; 72], Direction::ClientToServer, 1).is_err());
        assert_eq!(
            decrypt_application(&wire, &[1; 72], Direction::ServerToClient, 1)
                .unwrap()
                .body,
            body
        );
        assert!(
            encrypt_application(
                1,
                &vec![0; MAX_SERVER_APPLICATION_BODY_BYTES + 1],
                &[1; 72],
                Direction::ServerToClient,
                1,
                [2; 16]
            )
            .is_err()
        );
    }
}
