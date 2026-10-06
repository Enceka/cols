//! Demonware UDP address discovery (not RFC5389 packet framing).
//! Candidate shapes from AlphaBuild, checked against target-client traffic at runtime.
use std::net::SocketAddrV4;

#[derive(Debug, PartialEq, Eq)]
pub enum ReplySource {
    Received,
    Alternate,
}

pub fn reply(
    request: &[u8],
    mapped: SocketAddrV4,
    alternate: SocketAddrV4,
) -> Option<(Vec<u8>, ReplySource)> {
    if request.len() < 3 {
        return None;
    }
    let version = u16::from_le_bytes([request[1], request[2]]);
    let ip = mapped.ip().octets();
    let port = mapped.port().to_le_bytes();
    match request[0] {
        30 if request.len() == 3 && (version == 2 || version == 3) => {
            // Target VA 0xa7eb90 reads a plain address, then for v3 a second
            // address transformed by the XOR helper at 0xa7ead0.
            let mut out = vec![31, request[1], request[2]];
            out.extend_from_slice(&ip);
            out.extend_from_slice(&port);
            out.extend_from_slice(&(u32::from_le_bytes(ip) ^ 0x2112a442).to_le_bytes());
            out.extend_from_slice(&(mapped.port() ^ 0x2112).to_le_bytes());
            Some((out, ReplySource::Received))
        }
        20 if version == 2 && request.len() == 4 && [0, 2, 3].contains(&request[3]) => {
            let mut out = vec![21, 2, 0];
            out.extend_from_slice(&ip);
            out.extend_from_slice(&port);
            out.extend_from_slice(&alternate.ip().octets());
            out.extend_from_slice(&alternate.port().to_le_bytes());
            Some((
                out,
                if request[3] == 0 {
                    ReplySource::Received
                } else {
                    ReplySource::Alternate
                },
            ))
        }
        14 if version == 2 && request.len() == 29 => {
            let mut out = request.to_vec();
            out[0] = 15;
            out[18..22].copy_from_slice(&ip);
            out[22..24].copy_from_slice(&port);
            out[24..28].copy_from_slice(&ip);
            Some((out, ReplySource::Received))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_and_change_request_use_explicit_address_and_source() {
        let mapped = "127.0.0.1:12345".parse().unwrap();
        let alt = "127.0.0.1:3078".parse().unwrap();
        let (bytes, source) = reply(&[30, 2, 0], mapped, alt).unwrap();
        assert_eq!(
            bytes,
            [31, 2, 0, 127, 0, 0, 1, 57, 48, 61, 164, 18, 32, 43, 17]
        );
        assert_eq!(source, ReplySource::Received);
        let (bytes, source) = reply(&[20, 2, 0, 3], mapped, alt).unwrap();
        assert_eq!(&bytes[9..], [127, 0, 0, 1, 6, 12]);
        assert_eq!(source, ReplySource::Alternate);
        assert!(reply(&[20, 2, 0, 1], mapped, alt).is_none());
        assert_eq!(
            &reply(&[30, 3, 0], mapped, alt).unwrap().0[..3],
            &[31, 3, 0]
        );
        assert!(reply(&[30, 4, 0], mapped, alt).is_none());
        assert!(reply(&[30, 2], mapped, alt).is_none());
    }
}
