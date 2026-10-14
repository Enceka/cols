//! Offline inspection of a single complete TLS ClientHello record.
//! This is not a TLS implementation or TCP framer. Input must already be
//! reassembled; fragmented/multiple records are deliberately rejected.

use std::{error::Error, fmt};

pub const MAX_PLAINTEXT_RECORD: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHello {
    pub record_version: u16,
    pub client_version: u16,
    pub session_id: Vec<u8>,
    pub cipher_suites: Vec<u16>,
    pub compression_methods: Vec<u8>,
    pub extensions: Vec<Extension>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub kind: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub reason: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TLS offset {}: {}", self.offset, self.reason)
    }
}

impl Error for ParseError {}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn fail<T>(&self, reason: &'static str) -> Result<T, ParseError> {
        Err(ParseError {
            offset: self.pos,
            reason,
        })
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ParseError> {
        if count > self.remaining() {
            return self.fail("truncated field");
        }
        let start = self.pos;
        self.pos += count;
        Ok(&self.bytes[start..self.pos])
    }

    fn byte(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ParseError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u24(&mut self) -> Result<usize, ParseError> {
        let bytes = self.take(3)?;
        Ok((usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]))
    }
}

pub fn parse_client_hello(record: &[u8]) -> Result<ClientHello, ParseError> {
    let mut input = Cursor {
        bytes: record,
        pos: 0,
    };
    if input.byte()? != 22 {
        return input.fail("expected a handshake record");
    }
    let record_version = input.u16()?;
    let length = usize::from(input.u16()?);
    if length > MAX_PLAINTEXT_RECORD {
        return input.fail("plaintext record exceeds TLS limit");
    }
    if length != input.remaining() {
        return input.fail("expected exactly one complete TLS record");
    }
    if input.byte()? != 1 {
        return input.fail("expected ClientHello");
    }
    let body_len = input.u24()?;
    if body_len != input.remaining() {
        return input.fail("fragmented or extra handshake data is unsupported");
    }
    let client_version = input.u16()?;
    input.take(32)?; // Public random; not required for this summary.
    let session_len = usize::from(input.byte()?);
    if session_len > 32 {
        return input.fail("session ID exceeds TLS limit");
    }
    let session_id = input.take(session_len)?.to_vec();
    let cipher_len = usize::from(input.u16()?);
    if cipher_len == 0 || cipher_len % 2 != 0 {
        return input.fail("invalid cipher suite vector");
    }
    let cipher_suites = input
        .take(cipher_len)?
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    let compression_len = usize::from(input.byte()?);
    if compression_len == 0 {
        return input.fail("empty compression methods vector");
    }
    let compression_methods = input.take(compression_len)?.to_vec();
    let mut extensions = Vec::new();
    if input.remaining() > 0 {
        let extension_len = usize::from(input.u16()?);
        if extension_len != input.remaining() {
            return input.fail("invalid extensions length");
        }
        while input.remaining() > 0 {
            let kind = input.u16()?;
            let size = usize::from(input.u16()?);
            extensions.push(Extension {
                kind,
                data: input.take(size)?.to_vec(),
            });
        }
    }
    Ok(ClientHello {
        record_version,
        client_version,
        session_id,
        cipher_suites,
        compression_methods,
        extensions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const OBSERVED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/clienthello-3.3.3.3.bin"
    ));

    #[test]
    fn reads_the_real_client_hello() {
        let hello = parse_client_hello(OBSERVED).unwrap();
        assert_eq!(OBSERVED.len(), 70);
        assert_eq!(hello.record_version, 0x0301);
        assert_eq!(hello.client_version, 0x0301);
        assert!(hello.session_id.is_empty());
        assert_eq!(
            hello.cipher_suites,
            [
                0x39, 0x38, 0x35, 0x33, 0x32, 0x2f, 0x16, 0x13, 0x0a, 0x07, 0xff
            ]
        );
        assert_eq!(hello.compression_methods, [0]);
        assert!(hello.extensions.is_empty());
    }

    #[test]
    fn rejects_every_truncated_prefix_and_inconsistent_lengths() {
        for end in 0..OBSERVED.len() {
            assert!(parse_client_hello(&OBSERVED[..end]).is_err());
        }
        for (offset, value) in [(0, 23), (4, 66), (8, 60), (43, 33), (45, 23), (68, 0)] {
            let mut malformed = OBSERVED.to_vec();
            malformed[offset] = value;
            assert!(parse_client_hello(&malformed).is_err(), "offset {offset}");
        }
        let mut extra = OBSERVED.to_vec();
        extra.push(0);
        assert!(parse_client_hello(&extra).is_err());
    }
}
