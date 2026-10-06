//! One bounded HTTP/1.1 POST /auth/ exchange. No keep-alive or proxy semantics.
use codol_protocol::auth::MAX_AUTH_BODY;
use std::{error::Error, fmt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_HEADERS: usize = 8192;

#[derive(Debug)]
pub struct Request {
    pub wire: Vec<u8>,
    pub body: Vec<u8>,
    pub transaction_id: Option<String>,
}

#[derive(Debug)]
pub struct HttpError {
    pub status: u16,
    pub reason: &'static str,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.reason)
    }
}
impl Error for HttpError {}

fn error(status: u16, reason: &'static str) -> HttpError {
    HttpError { status, reason }
}

pub async fn read_auth<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<Request, HttpError> {
    let mut wire = Vec::new();
    let mut buffer = [0u8; 2048];
    let header_end = loop {
        if let Some(position) = wire.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
        if wire.len() >= MAX_HEADERS {
            return Err(error(431, "header limit exceeded"));
        }
        let available = buffer.len().min(MAX_HEADERS - wire.len());
        let count = stream
            .read(&mut buffer[..available])
            .await
            .map_err(|_| error(400, "read failed"))?;
        if count == 0 {
            return Err(error(400, "truncated headers"));
        }
        wire.extend_from_slice(&buffer[..count]);
    };
    let mut fields = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Request::new(&mut fields);
    if !parsed
        .parse(&wire[..header_end])
        .map_err(|_| error(400, "invalid headers"))?
        .is_complete()
    {
        return Err(error(400, "incomplete headers"));
    }
    if parsed.version != Some(1) {
        return Err(error(505, "HTTP/1.1 required"));
    }
    if parsed.method != Some("POST") {
        return Err(error(405, "POST required"));
    }
    if parsed.path != Some("/auth/") {
        return Err(error(404, "unknown route"));
    }
    let mut length = None;
    let mut expect = false;
    let mut content_type = false;
    let mut host = false;
    let mut transaction_id = None;
    for field in parsed.headers.iter() {
        let value = std::str::from_utf8(field.value)
            .map_err(|_| error(400, "invalid header encoding"))?
            .trim();
        if field.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(error(400, "transfer encoding is unsupported"));
        }
        if field.name.eq_ignore_ascii_case("content-length") {
            if length.is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(error(400, "ambiguous content length"));
            }
            let size: usize = value
                .parse()
                .map_err(|_| error(413, "body limit exceeded"))?;
            if size > MAX_AUTH_BODY {
                return Err(error(413, "body limit exceeded"));
            }
            length = Some(size);
        }
        if field.name.eq_ignore_ascii_case("expect") {
            if expect || !value.eq_ignore_ascii_case("100-continue") {
                return Err(error(417, "unsupported expectation"));
            }
            expect = true;
        }
        if field.name.eq_ignore_ascii_case("host") {
            if host || value.is_empty() {
                return Err(error(400, "invalid host"));
            }
            host = true;
        }
        if field.name.eq_ignore_ascii_case("content-type") {
            if content_type
                || !value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("application/json")
            {
                return Err(error(415, "application/json required"));
            }
            content_type = true;
        }
        if field.name.eq_ignore_ascii_case("x-transactionid") {
            if transaction_id.is_some()
                || value.is_empty()
                || value.len() > 20
                || !value.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(error(400, "invalid transaction ID"));
            }
            transaction_id = Some(value.to_owned());
        }
    }
    if !host {
        return Err(error(400, "host required"));
    }
    if !content_type {
        return Err(error(415, "content type required"));
    }
    let length = length.ok_or_else(|| error(411, "content length required"))?;
    if wire.len() > header_end + length {
        return Err(error(400, "extra bytes after body"));
    }
    if expect {
        stream
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .map_err(|_| error(400, "continue write failed"))?;
        stream
            .flush()
            .await
            .map_err(|_| error(400, "continue flush failed"))?;
    }
    while wire.len() < header_end + length {
        let available = buffer.len().min(header_end + length - wire.len());
        let count = stream
            .read(&mut buffer[..available])
            .await
            .map_err(|_| error(400, "read failed"))?;
        if count == 0 {
            return Err(error(400, "truncated body"));
        }
        wire.extend_from_slice(&buffer[..count]);
    }
    Ok(Request {
        body: wire[header_end..].to_vec(),
        wire,
        transaction_id,
    })
}

pub async fn write_response<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        417 => "Expectation Failed",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        505 => "HTTP Version Not Supported",
        _ => "Error",
    };
    stream.write_all(format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}
