//! Tagged task arguments and replies for the observed 3.3.3.3 services.
//! See docs/SERVICE_INITIALIZATION.md for serializers and response consumers.
use crate::lobby::LobbyError;

pub const MAX_FILE_BYTES: usize = 60 * 1024;
pub const NO_FILE: u32 = 1000;
/// Profile consumer VA 0x6d83e0 explicitly initializes defaults for code 800.
pub const NO_PROFILE: u32 = 800;
/// The target uses code 4 for task handling/deserialize failure. This is also
/// our explicit local failure for unsupported work; never claim empty success.
pub const TASK_FAILED: u32 = 4;

pub struct Request<'a> {
    pub service: u8,
    pub task: u8,
    pub arguments: &'a [u8],
}
impl<'a> Request<'a> {
    pub fn parse(body: &'a [u8]) -> Result<Self, LobbyError> {
        if body.len() < 3 || body[1] != 3 {
            return Err(LobbyError("expected service and tagged task"));
        }
        Ok(Self {
            service: body[0],
            task: body[2],
            arguments: &body[3..],
        })
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], LobbyError> {
        let value = self
            .bytes
            .get(..count)
            .ok_or(LobbyError("truncated task value"))?;
        self.bytes = &self.bytes[count..];
        Ok(value)
    }
    fn tag(&mut self, tag: u8) -> Result<(), LobbyError> {
        if self.take(1)? != [tag] {
            return Err(LobbyError("unexpected task value tag"));
        }
        Ok(())
    }
    pub fn u32(&mut self) -> Result<u32, LobbyError> {
        self.tag(8)?;
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| LobbyError("u32 size"))?,
        ))
    }
    pub fn u16(&mut self) -> Result<u16, LobbyError> {
        self.tag(6)?;
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| LobbyError("u16 size"))?,
        ))
    }
    pub fn i32(&mut self) -> Result<i32, LobbyError> {
        self.tag(7)?;
        Ok(i32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| LobbyError("i32 size"))?,
        ))
    }
    pub fn u64(&mut self) -> Result<u64, LobbyError> {
        self.tag(10)?;
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| LobbyError("u64 size"))?,
        ))
    }
    pub fn boolean(&mut self) -> Result<bool, LobbyError> {
        self.tag(1)?;
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(LobbyError("invalid task boolean")),
        }
    }
    pub fn string(&mut self, capacity: usize) -> Result<&'a str, LobbyError> {
        self.tag(16)?;
        let end = self
            .bytes
            .iter()
            .take(capacity)
            .position(|b| *b == 0)
            .ok_or(LobbyError("unterminated or oversized task string"))?;
        let value = self.take(end + 1)?;
        std::str::from_utf8(&value[..end])
            .map_err(|_| LobbyError("unsupported task string encoding"))
    }
    pub fn blob(&mut self) -> Result<&'a [u8], LobbyError> {
        self.tag(19)?;
        let size = self.u32()? as usize;
        if size > MAX_FILE_BYTES {
            return Err(LobbyError("task blob exceeds local limit"));
        }
        self.take(size)
    }
    pub fn finish(self) -> Result<(), LobbyError> {
        if self.bytes != [0] {
            return Err(LobbyError("expected task terminator without trailing data"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum StorageRequest<'a> {
    Read {
        context: &'a str,
        name: &'a str,
        owner: u64,
        account_type: &'a str,
    },
    Publisher {
        context: &'a str,
        name: &'a str,
    },
    Upload {
        context: &'a str,
        name: &'a str,
        visibility: bool,
        data: &'a [u8],
        owner: u64,
        account_type: &'a str,
        version: u32,
    },
}
pub fn storage<'a>(request: &Request<'a>) -> Result<StorageRequest<'a>, LobbyError> {
    if request.service != 10 {
        return Err(LobbyError("not a storage service request"));
    }
    let mut reader = Reader::new(request.arguments);
    let context = reader.string(16)?;
    let name = reader.string(128)?;
    let value = match request.task {
        12 => StorageRequest::Read {
            context,
            name,
            owner: reader.u64()?,
            account_type: reader.string(10)?,
        },
        21 => StorageRequest::Publisher { context, name },
        10 => StorageRequest::Upload {
            context,
            name,
            visibility: reader.boolean()?,
            data: reader.blob()?,
            owner: reader.u64()?,
            account_type: reader.string(10)?,
            version: reader.u32()?,
        },
        _ => return Err(LobbyError("unsupported storage task")),
    };
    reader.finish()?;
    Ok(value)
}

#[derive(Default)]
pub struct Writer(pub Vec<u8>);
impl Writer {
    pub fn u8(&mut self, value: u8) {
        self.0.extend_from_slice(&[3, value]);
    }
    pub fn u32(&mut self, value: u32) {
        self.0.push(8);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn u64(&mut self, value: u64) {
        self.0.push(10);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn boolean(&mut self, value: bool) {
        self.0.extend_from_slice(&[1, u8::from(value)]);
    }
    pub fn string(&mut self, value: &str) {
        self.0.push(16);
        self.0.extend_from_slice(value.as_bytes());
        self.0.push(0);
    }
    pub fn blob(&mut self, value: &[u8]) -> Result<(), LobbyError> {
        if value.len() > MAX_FILE_BYTES {
            return Err(LobbyError("file exceeds local limit"));
        }
        self.0.push(19);
        self.u32(value.len() as u32);
        self.0.extend_from_slice(value);
        Ok(())
    }
}

/// Normal kind-1 replies are consumed FIFO (VA 0xa3c090). The transaction ID
/// is assigned here and stored by the client, not echoed from the request.
pub fn reply(
    transaction: u64,
    task: u8,
    result: Result<&[Vec<u8>], u32>,
) -> Result<Vec<u8>, LobbyError> {
    let mut out = Writer::default();
    out.u64(transaction);
    match result {
        Err(0) => return Err(LobbyError("zero error code is not a failure")),
        Err(code) => out.u32(code),
        Ok(records) => {
            if records.len() > 2000 {
                return Err(LobbyError("too many task results"));
            }
            out.u32(0);
            out.u8(task);
            out.u32(records.len() as u32);
            if !records.is_empty() {
                out.u32(records.len() as u32);
                for record in records {
                    if out.0.len() + record.len() > 65000 {
                        return Err(LobbyError("task reply exceeds frame budget"));
                    }
                    out.0.extend_from_slice(record);
                }
            }
        }
    }
    Ok(out.0)
}

/// 80/230 single player-data result (VA 0xa4a6b0): account, balances,
/// inventory, then entitlement pairs (VA 0xa4bd80). A new local account has
/// no accepted grants/purchases. This does not seed game-specific starter items.
pub fn empty_player_data(user_id: u64) -> Vec<u8> {
    let mut out = Writer::default();
    out.u64(user_id);
    out.string("");
    out.u32(0); // Balance count.
    out.u32(0); // Inventory record count.
    if user_id == u64::MAX {
        // bdUserAccountID(-1, "") means no inherited identity to the nested reader.
        out.u64(user_id);
        out.string("");
    }
    out.u32(0); // Entitlement pair count is UInt32 on wire, UInt16 in memory.
    out.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_player_record_inherits_unsigned_identity_and_uses_u32_counts() {
        assert_eq!(
            empty_player_data(0x8000_0000_0000_0001),
            [
                10, 1, 0, 0, 0, 0, 0, 0, 128, 16, 0, 8, 0, 0, 0, 0, 8, 0, 0, 0, 0, 8, 0, 0, 0, 0
            ]
        );
        let sentinel = empty_player_data(u64::MAX);
        assert_eq!(&sentinel[21..32], &sentinel[..11]);
        assert_eq!(&sentinel[32..], &[8, 0, 0, 0, 0]);
    }
    #[test]
    fn target_read_shape_rejects_truncation_wrong_tags_and_extra_data() {
        let mut body = vec![10, 3, 12];
        let mut args = Writer::default();
        args.string("");
        args.string("the.json");
        args.u64(u64::MAX);
        args.string("");
        args.0.push(0);
        body.extend_from_slice(&args.0);
        assert!(matches!(
            storage(&Request::parse(&body).unwrap()).unwrap(),
            StorageRequest::Read {
                owner: u64::MAX,
                name: "the.json",
                ..
            }
        ));
        for n in 0..body.len() {
            assert!(
                Request::parse(&body[..n])
                    .and_then(|r| storage(&r).map(|_| ()))
                    .is_err()
            );
        }
        body.push(0);
        assert!(storage(&Request::parse(&body).unwrap()).is_err());
        body.pop();
        body[3] = 19;
        assert!(storage(&Request::parse(&body).unwrap()).is_err());
        assert!(Reader::new(&[19, 8, 255, 255, 255, 255]).blob().is_err());
    }
    #[test]
    fn response_envelopes_match_independently_written_wire_bytes() {
        assert_eq!(
            reply(1, 21, Err(NO_FILE)).unwrap(),
            [10, 1, 0, 0, 0, 0, 0, 0, 0, 8, 232, 3, 0, 0]
        );
        assert_eq!(
            reply(2, 6, Ok(&[vec![8, 0, 241, 83, 101]])).unwrap(),
            [
                10, 2, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 3, 6, 8, 1, 0, 0, 0, 8, 1, 0, 0, 0, 8,
                0, 241, 83, 101
            ]
        );
        assert_eq!(
            reply(1, 4, Ok(&[])).unwrap(),
            crate::lobby::registration_status(1)
        );
        assert!(reply(1, 4, Err(0)).is_err());
        assert!(reply(1, 4, Ok(&[vec![0; 65000]])).is_err());
    }
}
