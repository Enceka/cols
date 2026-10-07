//! Application connection bootstrap after authenticated game DATA.
use anyhow::{Result, ensure};

#[derive(Default)]
pub(crate) struct Connection {
    challenge: Option<u32>,
    connected: bool,
    stats: codol_protocol::game_stats::Upload,
    eof_probe_sent: bool,
    reliable_probe_sent: bool,
}

impl Connection {
    pub fn stage(&self) -> &'static str {
        if self.stats.profile().is_some() {
            "stats_received"
        } else if self.connected {
            "connected"
        } else if self.challenge.is_some() {
            "challenged"
        } else {
            "unconnected"
        }
    }

    pub fn challenge_value(&self) -> Option<u32> {
        self.challenge
    }

    pub fn handle(&mut self, payload: &[u8], owner: u64, build: i32) -> Result<Option<Vec<u8>>> {
        let Some(command) = payload.strip_prefix(&[255; 4]) else {
            if payload.len() >= 14
                && self.connected
                && let Some(challenge) = self.challenge
            {
                let (_, clear) = codol_protocol::game_wire::open_client(payload, challenge, &[])?;
                if clear == [3] {
                    if !self.eof_probe_sent {
                        self.eof_probe_sent = true;
                        return Ok(Some(codol_protocol::game_wire::server_packet(
                            1,
                            0,
                            challenge,
                            &[],
                            &[6],
                        )?));
                    }
                    let (header, _) =
                        codol_protocol::game_wire::open_client(payload, challenge, &[])?;
                    if header.message_ack >= 1 && !self.reliable_probe_sent {
                        self.reliable_probe_sent = true;
                        let mut body = vec![1];
                        body.extend_from_slice(&1u32.to_le_bytes());
                        body.extend_from_slice(b"d 3 1\0");
                        return Ok(Some(codol_protocol::game_wire::server_packet(
                            2,
                            1,
                            challenge,
                            &[],
                            &body,
                        )?));
                    }
                }
            }
            // Inner packets are captured for netchan/gamestate recovery. Do
            // not answer with an unverified gamestate.
            return Ok(None);
        };
        if let Some(chunk) = command.strip_prefix(b"stats\0") {
            ensure!(self.connected, "stats before connect");
            let missing = self.stats.accept(chunk)?;
            let mut reply = vec![255; 4];
            reply.extend_from_slice(format!("statResponse {missing}\0").as_bytes());
            return Ok(Some(reply));
        }
        if command.strip_suffix(&[0]).unwrap_or(command) == b"heartbeat" {
            ensure!(
                self.stats.profile().is_some(),
                "heartbeat before stats completion"
            );
            // Target 0x466c30 accepts this only in state 6 and advances when
            // the local map-loading worker has completed (0x4640a0).
            return Ok(Some(b"\xff\xff\xff\xffheartbeatresponse\0".to_vec()));
        }
        if let Some(connect) = command.strip_prefix(b"connect ") {
            let challenge = self
                .challenge
                .ok_or_else(|| anyhow::anyhow!("connect before challenge"))?;
            validate_connect(connect, owner, build, challenge)?;
            self.connected = true;
            // 0x466b24..0x466c22 initializes netchan, enters state 5 and starts
            // uploading stats. This target does not consume an info-string here.
            return Ok(Some(b"\xff\xff\xff\xffconnectResponse\0".to_vec()));
        }
        if !command.starts_with(b"getchallenge ") {
            return Ok(None);
        }
        ensure!(command.len() <= 128, "game challenge request too long");
        let command = command.strip_suffix(&[0]).unwrap_or(command);
        let command = std::str::from_utf8(command)?;
        let fields = command.split(' ').collect::<Vec<_>>();
        ensure!(
            fields.len() == 3 && fields[1] == "0",
            "unsupported challenge request"
        );
        let uid = fields[2]
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(|| anyhow::anyhow!("missing challenge identity"))?;
        ensure!(
            uid.len() == 16 && uid.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid challenge identity"
        );
        ensure!(
            u64::from_str_radix(uid, 16)? == owner,
            "challenge identity differs from admission"
        );
        let challenge = if let Some(value) = self.challenge {
            value
        } else {
            let mut random = [0; 4];
            openssl::rand::rand_bytes(&mut random)?;
            let value = (u32::from_le_bytes(random) & i32::MAX as u32).max(1);
            self.challenge = Some(value);
            value
        };
        // Target 0x466a55 reads argv(1) as an integer into connection +0x138,
        // then changes state from challenging (3) to connecting (4).
        let mut reply = vec![255; 4];
        reply.extend_from_slice(format!("challengeResponse {challenge}\0").as_bytes());
        Ok(Some(reply))
    }
}

fn identity(bytes: &[u8]) -> Result<u64> {
    ensure!(
        bytes.len() == 16 && bytes.iter().all(u8::is_ascii_hexdigit),
        "invalid connect identity"
    );
    Ok(u64::from_str_radix(std::str::from_utf8(bytes)?, 16)?)
}

fn validate_connect(body: &[u8], owner: u64, build: i32, challenge: u32) -> Result<()> {
    ensure!(body.len() <= 1024, "connect request too long");
    let body = body.strip_suffix(&[0]).unwrap_or(body);
    ensure!(
        body.len() >= 20 && body[16] == b' ',
        "invalid connect framing"
    );
    ensure!(
        identity(&body[..16])? == owner,
        "connect identity differs from admission"
    );
    let info = body[17..]
        .strip_prefix(b"\"")
        .and_then(|s| s.strip_suffix(b"\""))
        .ok_or_else(|| anyhow::anyhow!("connect userinfo must be quoted"))?;
    ensure!(
        !info.iter().any(|b| *b < 32 || *b == 127 || *b == b'"'),
        "invalid connect userinfo character"
    );
    let info = info
        .strip_prefix(b"\\")
        .ok_or_else(|| anyhow::anyhow!("invalid userinfo prefix"))?;
    let mut fields = std::collections::BTreeMap::<&[u8], &[u8]>::new();
    let mut parts = info.split(|b| *b == b'\\');
    while let Some(key) = parts.next() {
        let value = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("unpaired userinfo field"))?;
        ensure!(
            !key.is_empty()
                && key.len() <= 64
                && key.is_ascii()
                && value.len() <= 512
                && fields.len() < 64,
            "userinfo field limit"
        );
        ensure!(
            fields.insert(key, value).is_none(),
            "duplicate userinfo field"
        );
    }
    let get = |key: &[u8]| -> Result<&[u8]> {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("missing required connect field"))
    };
    ensure!(
        identity(get(b"xuid")?)? == owner,
        "userinfo identity differs from admission"
    );
    ensure!(
        std::str::from_utf8(get(b"challenge")?)?.parse::<u32>()? == challenge,
        "connect challenge mismatch"
    );
    ensure!(
        std::str::from_utf8(get(b"2490")?)?.parse::<i32>()? == build,
        "connect build mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stats_and_heartbeat_require_prior_stages_and_all_fragments() {
        let mut connection = Connection::default();
        let heartbeat = b"\xff\xff\xff\xffheartbeat\0";
        assert!(connection.handle(heartbeat, 42, 7).is_err());
        let raw = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (index, data) in raw
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut packet = b"\xff\xff\xff\xffstats\0".to_vec();
            packet.extend_from_slice(&[index as u8, 0, 0]);
            packet.extend_from_slice(data);
            if index == 0 {
                assert!(connection.handle(&packet, 42, 7).is_err());
                connection.connected = true;
            }
            let reply = connection.handle(&packet, 42, 7).unwrap().unwrap();
            if index < 26 {
                assert_ne!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
                assert!(connection.handle(heartbeat, 42, 7).is_err());
            } else {
                assert_eq!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
            }
        }
        assert_eq!(connection.stage(), "stats_received");
        assert_eq!(
            connection.handle(heartbeat, 42, 7).unwrap().unwrap(),
            b"\xff\xff\xff\xffheartbeatresponse\0"
        );
    }
    #[test]
    fn challenge_requires_admitted_identity_and_retries_reuse_nonce() {
        let mut connection = Connection::default();
        let request = b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\"\0";
        assert!(connection.handle(request, 1, 7).is_err());
        assert!(connection.challenge.is_none());
        let response = connection
            .handle(request, u64::MAX - 1, 7)
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"\xff\xff\xff\xffchallengeResponse "));
        assert_eq!(
            connection
                .handle(request, u64::MAX - 1, 7)
                .unwrap()
                .unwrap(),
            response
        );
        assert!(
            connection
                .handle(
                    b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\" trailing",
                    u64::MAX - 1,
                    7
                )
                .is_err()
        );
        assert!(
            connection
                .handle(&[0, 0], u64::MAX - 1, 7)
                .unwrap()
                .is_none()
        );
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffarbitrary", u64::MAX - 1, 7)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn connect_binds_both_identities_build_and_nonce_without_duplicate_fields() {
        let mut connection = Connection::default();
        let valid = b"\xff\xff\xff\xffconnect fffffffffffffffe \"\\name\\Lab player\\xuid\\fffffffffffffffe\\challenge\\12345\\2490\\-688906341\"";
        assert!(connection.handle(valid, u64::MAX - 1, -688906341).is_err());
        connection.challenge = Some(12345);
        for (old, new) in [
            ("fffffffffffffffe", "ffffffffffffffff"),
            ("12345", "12346"),
            ("-688906341", "0"),
            ("\\name\\", "\\xuid\\"),
        ] {
            let text = std::str::from_utf8(&valid[4..]).unwrap().replace(old, new);
            let mut damaged = vec![255; 4];
            damaged.extend_from_slice(text.as_bytes());
            assert!(
                connection
                    .handle(&damaged, u64::MAX - 1, -688906341)
                    .is_err()
            );
            assert!(!connection.connected);
        }
        for _ in 0..2 {
            assert_eq!(
                connection
                    .handle(valid, u64::MAX - 1, -688906341)
                    .unwrap()
                    .unwrap(),
                b"\xff\xff\xff\xffconnectResponse\0"
            );
        }
        assert_eq!(connection.stage(), "connected");
        let mut extra = valid.to_vec();
        extra.extend_from_slice(b" garbage");
        assert!(connection.handle(&extra, u64::MAX - 1, -688906341).is_err());
    }

    #[test]
    fn first_client_eof_gets_only_a_native_server_eof_probe() {
        let mut connection = Connection {
            challenge: Some(0x75),
            connected: true,
            ..Default::default()
        };
        let packet = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x76];
        let reply = connection.handle(&packet, 1, 7).unwrap().unwrap();
        assert_eq!(&reply[..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
        let mut next = packet;
        next[5..9].copy_from_slice(&1u32.to_le_bytes());
        next[13] = 0x77; // 0x77 ^ challenge(0x75) ^ messageAck(1) = clc_EOF(3)
        let second = connection.handle(&next, 1, 7).unwrap().unwrap();
        assert_eq!(&second[..8], &[2, 0, 0, 0, 1, 0, 0, 0]);
        assert!(connection.handle(&next, 1, 7).unwrap().is_none());
    }
}
