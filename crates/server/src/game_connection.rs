//! Application connection bootstrap after authenticated game DATA.
use anyhow::{Result, ensure};

#[derive(Default)]
pub(crate) struct Connection {
    challenge: Option<u32>,
}

impl Connection {
    pub fn handle(&mut self, payload: &[u8], owner: u64) -> Result<Option<Vec<u8>>> {
        let Some(command) = payload.strip_prefix(&[255; 4]) else {
            return Ok(None);
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn challenge_requires_admitted_identity_and_retries_reuse_nonce() {
        let mut connection = Connection::default();
        let request = b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\"\0";
        assert!(connection.handle(request, 1).is_err());
        assert!(connection.challenge.is_none());
        let response = connection.handle(request, u64::MAX - 1).unwrap().unwrap();
        assert!(response.starts_with(b"\xff\xff\xff\xffchallengeResponse "));
        assert_eq!(
            connection.handle(request, u64::MAX - 1).unwrap().unwrap(),
            response
        );
        assert!(
            connection
                .handle(
                    b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\" trailing",
                    u64::MAX - 1
                )
                .is_err()
        );
        assert!(connection.handle(&[0, 0], u64::MAX - 1).unwrap().is_none());
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffarbitrary", u64::MAX - 1)
                .unwrap()
                .is_none()
        );
    }
}
