//! Publish public lobby chat text from a scripted identity.
//! Obtains a local test ticket from the experiment AUTH endpoint, opens an
//! encrypted lobby session and sends the public-chat frames a native client
//! would send, so a running client can be observed receiving public text.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::auth::ticket::{TICKET_SIZE, decrypt_ticket};
use codol_protocol::lobby::channel as lobby;
use codol_protocol::lobby::task::{Reader, Writer};
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode, SslVersion};
use serde_json::{Value, json};
use std::{net::SocketAddr, pin::Pin, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_openssl::SslStream;

const TITLE_ID: u32 = 1002;
const GROUP_TYPE: u8 = 130;
/// Group id the native send branch uses for public text.
const PUBLIC_GROUP: u64 = 10;
/// Group id the native subscription builder passes as `ServerChannel.NEWBIE`.
const NEWBIE_GROUP: u64 = 99;
/// Public text carries the tutor marker the native client strips before display.
const MARKER: &str = "tUtoR0";
const MAX_TEXT: usize = 1024;

struct Arguments {
    auth: SocketAddr,
    lobby: SocketAddr,
    user: u64,
    iv_seed: u32,
    group: u64,
    text: String,
    subscribe: bool,
}

fn arguments() -> Result<Arguments> {
    let mut auth = None;
    let mut lobby = None;
    let mut user = None;
    let mut iv_seed = 1;
    let mut group = PUBLIC_GROUP;
    let mut text = None;
    let mut subscribe = false;
    let mut values = std::env::args().skip(1);
    while let Some(key) = values.next() {
        match key.as_str() {
            "--auth" => auth = Some(values.next().context("--auth value")?.parse()?),
            "--lobby" => lobby = Some(values.next().context("--lobby value")?.parse()?),
            "--user" => user = Some(values.next().context("--user value")?.parse()?),
            "--iv-seed" => iv_seed = values.next().context("--iv-seed value")?.parse()?,
            "--group" => group = values.next().context("--group value")?.parse()?,
            "--text" => text = Some(values.next().context("--text value")?),
            "--subscribe" => subscribe = true,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    Ok(Arguments {
        auth: auth.context("--auth is required")?,
        lobby: lobby.context("--lobby is required")?,
        user: user.context("--user is required")?,
        iv_seed,
        group,
        text: text.context("--text is required")?,
        subscribe,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = arguments()?;
    let ticket = auth_ticket(&arguments).await?;
    let plain = decrypt_ticket(&ticket, arguments.iv_seed)?;
    let user = u64::from_le_bytes(plain[25..33].try_into().unwrap());
    ensure!(user == arguments.user, "ticket belongs to another account");
    let key: [u8; 24] = plain[97..121].try_into().unwrap();
    let mut peer = Peer::connect(arguments.lobby, &ticket, &key, arguments.iv_seed).await?;
    println!("lobby session established for user {user}");
    let (error, _, _) = peer.rpc(&join_body()).await?;
    println!("join attempt returned error {error}");
    if arguments.subscribe {
        let (error, count, _) = peer
            .rpc(&groups_body(&[(GROUP_TYPE, NEWBIE_GROUP)]))
            .await?;
        println!("subscribed to {GROUP_TYPE}/{NEWBIE_GROUP}: error {error}, {count} records");
    }
    let (error, count, _) = peer
        .rpc(&text_body(arguments.group, &arguments.text)?)
        .await?;
    ensure!(error == 0, "public text rejected with error {error}");
    println!(
        "public text accepted for group {}; reply carries {count} records",
        arguments.group
    );
    Ok(())
}

async fn auth_ticket(arguments: &Arguments) -> Result<[u8; TICKET_SIZE]> {
    let body = serde_json::to_vec(&json!({
        "auth_task": "10",
        "iv_seed": arguments.iv_seed.to_string(),
        "title_id": TITLE_ID.to_string(),
        "identity": "356c4bc3",
        "extra_data": json!({"version": "0", "user_id": arguments.user.to_string()}).to_string(),
    }))?;
    let mut stream = connect(arguments.auth).await?;
    let head = format!(
        "POST /auth/ HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let start = response
        .windows(4)
        .position(|value| value == b"\r\n\r\n")
        .context("AUTH response headers")?
        + 4;
    let reply: Value = serde_json::from_slice(&response[start..]).context("AUTH response body")?;
    ensure!(
        reply["auth_task"] == "11",
        "AUTH did not issue a ticket: {}",
        String::from_utf8_lossy(&response[start..])
    );
    let decoded = STANDARD.decode(reply["client_ticket"].as_str().context("client ticket")?)?;
    let ticket: [u8; TICKET_SIZE] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("AUTH ticket size"))?;
    Ok(ticket)
}

async fn connect(address: SocketAddr) -> Result<SslStream<TcpStream>> {
    let mut builder = SslConnector::builder(SslMethod::tls_client())?;
    builder.set_min_proto_version(Some(SslVersion::TLS1))?;
    builder.set_max_proto_version(Some(SslVersion::TLS1))?;
    builder.set_security_level(0);
    builder.set_cipher_list("AES128-SHA")?;
    // The experiment AUTH endpoint presents the lab certificate and never leaves
    // loopback, so peer verification stays off for this tool.
    builder.set_verify(SslVerifyMode::NONE);
    let ssl = builder.build().configure()?.into_ssl("localhost")?;
    let mut stream = SslStream::new(ssl, TcpStream::connect(address).await?)?;
    Pin::new(&mut stream).connect().await?;
    Ok(stream)
}

struct Peer {
    stream: TcpStream,
    keys: [u8; 72],
    incoming: u32,
    outgoing: u32,
    transaction: u64,
}

impl Peer {
    async fn connect(
        address: SocketAddr,
        ticket: &[u8; TICKET_SIZE],
        key: &[u8; 24],
        iv_seed: u32,
    ) -> Result<Self> {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/lobby210-synthetic.json"
        )))?;
        let hello: [u8; 28] = hex(fixture["client_hello"].as_str().context("client hello")?)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("client hello size"))?;
        let mut stream = TcpStream::connect(address).await?;
        stream.write_all(&hello).await?;
        let server = read_frame(&mut stream).await?;
        let mut encoded = Vec::new();
        let mut position = 0;
        let mut put = |value: u64, count: usize| {
            for bit in 0..count {
                if position / 8 == encoded.len() {
                    encoded.push(0);
                }
                encoded[position / 8] |= (((value >> bit) & 1) as u8) << (position % 8);
                position += 1;
            }
        };
        put(7, 8);
        put(1, 1);
        put(8, 5);
        put(u64::from(TITLE_ID), 32);
        put(8, 5);
        put(u64::from(iv_seed), 32);
        for byte in ticket {
            put(u64::from(*byte), 8);
        }
        let mut auth = lobby::frame(0x82, &[encoded, vec![0; 8]].concat());
        let secrets = lobby::derive_secrets(&hello, &server, &auth, key)?;
        let end = auth.len();
        auth[end - 8..].copy_from_slice(&secrets.client_proof);
        stream.write_all(&auth).await?;
        let ack = read_frame(&mut stream).await?;
        ensure!(
            ack == lobby::frame(0x83, &secrets.server_proof),
            "lobby handshake rejected"
        );
        Ok(Self {
            stream,
            keys: secrets.data_keys,
            incoming: 1,
            outgoing: 1,
            transaction: 1,
        })
    }
    async fn rpc(&mut self, body: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
        let wire = lobby::encrypt_application(
            0x86,
            body,
            &self.keys,
            lobby::Direction::ClientToServer,
            self.outgoing,
            [self.outgoing as u8; 16],
        )?;
        self.outgoing += 1;
        self.stream.write_all(&wire).await?;
        let wire = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.stream))
            .await
            .context("lobby reply timeout")??;
        let message = lobby::decrypt_application(
            &wire,
            &self.keys,
            lobby::Direction::ServerToClient,
            self.incoming,
        )?;
        self.incoming += 1;
        ensure!(
            message.kind == 1,
            "unexpected lobby reply kind {}",
            message.kind
        );
        let mut reader = Reader::new(&message.body);
        ensure!(reader.u64()? == self.transaction, "lobby reply transaction");
        self.transaction += 1;
        let error = reader.u32()?;
        // Failure replies carry the code and nothing else.
        if error != 0 {
            return Ok((error, 0, reader.remaining().to_vec()));
        }
        ensure!(reader.u8()? == body[2], "lobby reply task");
        let count = reader.u32()?;
        if count > 0 {
            ensure!(reader.u32()? == count, "lobby reply result count");
        }
        Ok((error, count, reader.remaining().to_vec()))
    }
}

async fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await?;
    let length = u32::from_le_bytes(prefix) as usize;
    ensure!(length <= 65536, "lobby frame exceeds research limit");
    let mut bytes = vec![0; length + 4];
    bytes[..4].copy_from_slice(&prefix);
    stream.read_exact(&mut bytes[4..]).await?;
    Ok(bytes)
}

/// Join names the session: the lobby registers the stored nickname before
/// room admission, so a failed join still gives the sender its chat name.
fn join_body() -> Vec<u8> {
    let mut writer = Writer::default();
    writer.0.extend_from_slice(&[11, 3, 11]);
    writer.u64(0);
    writer.u64(0);
    writer.0.push(0);
    writer.0
}

fn groups_body(added: &[(u8, u64)]) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.0.extend_from_slice(&[102, 3, 1]);
    writer.u32(0);
    writer.u32(added.len() as u32);
    for (kind, id) in added {
        writer.u8(*kind);
        writer.u64(*id);
    }
    writer.0.push(0);
    writer.0
}

fn text_body(group: u64, text: &str) -> Result<Vec<u8>> {
    let body = format!("{MARKER}{text}");
    ensure!(
        body.len() > MARKER.len() && body.len() <= MAX_TEXT,
        "text must be 1..={MAX_TEXT} bytes"
    );
    let mut payload = Vec::with_capacity(body.len() + 8);
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(body.len() as u32).to_le_bytes());
    payload.extend_from_slice(body.as_bytes());
    let mut writer = Writer::default();
    writer.0.extend_from_slice(&[102, 3, 3]);
    writer.u8(GROUP_TYPE);
    writer.u64(group);
    writer.blob(&payload)?;
    writer.0.push(0);
    Ok(writer.0)
}

fn hex(text: &str) -> Result<Vec<u8>> {
    ensure!(text.len().is_multiple_of(2), "odd hex length");
    (0..text.len())
        .step_by(2)
        .map(|at| Ok(u8::from_str_radix(&text[at..at + 2], 16)?))
        .collect()
}
