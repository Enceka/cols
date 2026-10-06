//! P-224 handshake for the observed 16-byte INIT, version 2.
//! Protocol facts researched against the target and AlphaBuild's network code.
use anyhow::{Result, ensure};
use codol_protocol::game::{Records, derive_key};
use openssl::{
    bn::{BigNum, BigNumContext},
    derive::Deriver,
    ec::{EcGroup, EcKey, EcPoint},
    nid::Nid,
    pkey::{PKey, Private},
};
use std::net::SocketAddrV4;

pub(crate) struct Peer {
    pub connection: crate::game_connection::Connection,
    pub id: [u8; 8],
    init: [u8; 16],
    init_ack: [u8; 38],
    key: PKey<Private>,
    public: [u8; 100],
    echo: Option<Vec<u8>>,
    cookie_ack: Option<Vec<u8>>,
    pub records: Option<Records>,
    pub local_tag: u16,
    pub remote_tag: u16,
}
pub(super) fn encode_public(key: &EcKey<Private>) -> Result<[u8; 100]> {
    let mut ctx = BigNumContext::new()?;
    let mut x = BigNum::new()?;
    let mut y = BigNum::new()?;
    key.public_key()
        .affine_coordinates_gfp(key.group(), &mut x, &mut y, &mut ctx)?;
    let mut der = vec![0x30, 0, 3, 2, 7, 0, 2, 1, 28];
    for value in [&x, &y] {
        let bytes = value.to_vec_padded(28)?;
        let sign = bytes[0] & 128 != 0;
        der.extend_from_slice(&[2, 28 + u8::from(sign)]);
        if sign {
            der.push(0);
        }
        der.extend_from_slice(&bytes);
    }
    der[1] = (der.len() - 2) as u8;
    let mut out = [0; 100];
    out[..der.len()].copy_from_slice(&der);
    Ok(out)
}
pub(super) fn shared_secret(key: &PKey<Private>, public: &[u8]) -> Result<[u8; 28]> {
    ensure!(
        public.len() == 100 && public[..9] == [0x30, public[1], 3, 2, 7, 0, 2, 1, 28],
        "invalid game public-key header"
    );
    let end = usize::from(public[1]) + 2;
    ensure!(
        (15..=71).contains(&end) && public[end..].iter().all(|b| *b == 0),
        "invalid game public-key envelope"
    );
    let mut at = 9;
    let mut coordinates = Vec::new();
    for _ in 0..2 {
        ensure!(
            at + 2 <= end && public[at] == 2,
            "invalid game coordinate tag"
        );
        let n = usize::from(public[at + 1]);
        at += 2;
        ensure!(
            (1..=29).contains(&n) && at + n <= end && public[at] & 128 == 0,
            "invalid game coordinate size/sign"
        );
        if n == 29 {
            ensure!(
                public[at] == 0 && public[at + 1] & 128 != 0,
                "invalid game coordinate padding"
            );
        }
        coordinates.push(BigNum::from_slice(&public[at..at + n])?);
        at += n;
    }
    ensure!(at == end, "extra game public-key fields");
    let group = EcGroup::from_curve_name(Nid::SECP224R1)?;
    let mut ctx = BigNumContext::new()?;
    let mut point = EcPoint::new(&group)?;
    point.set_affine_coordinates_gfp(&group, &coordinates[0], &coordinates[1], &mut ctx)?;
    let peer = EcKey::from_public_key(&group, &point)?;
    peer.check_key()?;
    let peer = PKey::from_ec_key(peer)?;
    let mut d = Deriver::new(key)?;
    d.set_peer(&peer)?;
    let x = d.derive_to_vec()?;
    ensure!(x.len() == 28, "unexpected game shared-secret width");
    x.try_into()
        .map_err(|_| anyhow::anyhow!("game secret size"))
}
impl Peer {
    pub fn init(packet: &[u8], address: SocketAddrV4) -> Result<Self> {
        ensure!(
            packet.len() == 16 && packet[..6] == [1, 2, 0, 0, 0, 0],
            "unsupported game INIT"
        );
        let remote_tag = u16::from_le_bytes([packet[6], packet[7]]);
        ensure!(remote_tag != 0, "zero INIT tag");
        let group = EcGroup::from_curve_name(Nid::SECP224R1)?;
        let ec = EcKey::generate(&group)?;
        let public = encode_public(&ec)?;
        let key = PKey::from_ec_key(ec)?;
        let mut random = [0; 2];
        openssl::rand::rand_bytes(&mut random)?;
        let local_tag = u16::from_le_bytes(random).max(1);
        let id: [u8; 8] = packet[8..16].try_into()?;
        let mut init_ack = [0u8; 38];
        init_ack[..2].copy_from_slice(&[2, 2]);
        init_ack[2..4].copy_from_slice(&remote_tag.to_le_bytes());
        init_ack[14..16].copy_from_slice(&local_tag.to_le_bytes());
        init_ack[16..18].copy_from_slice(&local_tag.to_le_bytes());
        init_ack[18..20].copy_from_slice(&remote_tag.to_le_bytes());
        init_ack[24..28].copy_from_slice(&address.ip().octets());
        init_ack[28..30].copy_from_slice(&address.port().to_le_bytes());
        init_ack[30..38].copy_from_slice(&id);
        Ok(Self {
            connection: Default::default(),
            id,
            init: packet.try_into()?,
            init_ack,
            key,
            public,
            echo: None,
            cookie_ack: None,
            records: None,
            local_tag,
            remote_tag,
        })
    }
    pub fn is_retry(&self, packet: &[u8]) -> bool {
        self.init == packet
    }
    pub fn init_reply(&self) -> Vec<u8> {
        self.init_ack.to_vec()
    }
    pub fn cookie(&mut self, packet: &[u8], security: &[u8; 16]) -> Result<Vec<u8>> {
        ensure!(
            packet.len() == 189
                && packet[..2] == [3, 2]
                && packet[2..4] == self.local_tag.to_le_bytes()
                && packet[4..6] == [0, 0]
                && packet[6..44] == self.init_ack
                && packet[81..89] == self.id,
            "game cookie/session mismatch"
        );
        if let Some(old) = &self.echo {
            ensure!(old == packet, "conflicting game cookie retry");
            return Ok(self.cookie_ack.clone().expect("established ACK"));
        }
        let x = shared_secret(&self.key, &packet[89..189])?;
        let key = derive_key(&x, security);
        let mut ack = vec![0; 114];
        ack[..2].copy_from_slice(&[4, 2]);
        ack[2..4].copy_from_slice(&self.remote_tag.to_le_bytes());
        ack[6..106].copy_from_slice(&self.public);
        ack[106..].copy_from_slice(&self.id);
        self.records = Some(Records::new(key));
        self.echo = Some(packet.to_vec());
        self.cookie_ack = Some(ack.clone());
        Ok(ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exchange_binds_cookie_peer_and_session_and_retransmits_identically() {
        let mut init = vec![1, 2, 0, 0, 0, 0, 42, 0];
        init.extend_from_slice(&[19; 8]);
        let mut server = Peer::init(&init, "127.0.0.1:30001".parse().unwrap()).unwrap();
        let group = EcGroup::from_curve_name(Nid::SECP224R1).unwrap();
        let ec = EcKey::generate(&group).unwrap();
        let public = encode_public(&ec).unwrap();
        let client = PKey::from_ec_key(ec).unwrap();
        let mut echo = vec![3, 2];
        echo.extend_from_slice(&server.local_tag.to_le_bytes());
        echo.extend_from_slice(&[0, 0]);
        echo.extend_from_slice(&server.init_reply());
        echo.extend_from_slice(&[0; 37]);
        echo.extend_from_slice(&[19; 8]);
        echo.extend_from_slice(&public);
        let mut bad = echo.clone();
        bad[81] ^= 1;
        assert!(server.cookie(&bad, &[7; 16]).is_err());
        assert!(server.records.is_none());
        let ack = server.cookie(&echo, &[7; 16]).unwrap();
        assert_eq!(server.cookie(&echo, &[7; 16]).unwrap(), ack);
        let secret = shared_secret(&client, &ack[6..106]).unwrap();
        let mut client = Records::new(derive_key(&secret, &[7; 16]));
        let data = client
            .seal(b"\xff\xff\xff\xffgetchallenge 123", server.local_tag)
            .unwrap();
        assert_eq!(
            server
                .records
                .as_mut()
                .unwrap()
                .open(&data, server.local_tag)
                .unwrap()
                .encrypted,
            b"\xff\xff\xff\xffgetchallenge 123"
        );
        bad = echo;
        bad[100] ^= 1;
        assert!(server.cookie(&bad, &[7; 16]).is_err());
    }

    #[test]
    fn p224_public_encoding_matches_alpha_reference_vector() {
        let group = EcGroup::from_curve_name(Nid::SECP224R1).unwrap();
        let private = BigNum::from_slice(&(1u8..=28).collect::<Vec<_>>()).unwrap();
        let mut context = BigNumContext::new().unwrap();
        let mut point = EcPoint::new(&group).unwrap();
        point
            .mul_generator2(&group, &private, &mut context)
            .unwrap();
        let ec = EcKey::from_private_components(&group, &private, &point).unwrap();
        let actual = encode_public(&ec).unwrap();
        let hex = "30440302070002011c021c627b7c0b3a2fb7a478ac5670e9973194a5fda0bc0791b07506a73ddd021d0099113b3fdea71bbff9921330d9ce980155eebd620c46be927c214543";
        let expected = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(&actual[..expected.len()], expected.as_slice());
        assert!(actual[expected.len()..].iter().all(|b| *b == 0));
    }
}
