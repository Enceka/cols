//! Tagged task arguments and replies for the observed 3.3.3.3 services.
//! See docs/SERVICE_INITIALIZATION.md for serializers and response consumers.
use crate::lobby::channel::LobbyError;

pub const MAX_FILE_BYTES: usize = 60 * 1024;
/// The target player-data decoder (0xa4a6b0) uses a dynamic inventory array.
/// 8192 records cover the 6772-item catalogue within the 512 KiB reply frame.
pub const MAX_INVENTORY_ITEMS: usize = 8192;
pub const MAX_TASK_RESULTS: usize = 8192;
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
    pub fn u8(&mut self) -> Result<u8, LobbyError> {
        self.tag(3)?;
        Ok(self.take(1)?[0])
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
    pub fn f32_bits(&mut self) -> Result<u32, LobbyError> {
        self.tag(13)?;
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| LobbyError("float size"))?,
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
    pub fn remaining(&self) -> &'a [u8] {
        self.bytes
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
    pub fn u16(&mut self, value: u16) {
        self.0.push(6);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn i64(&mut self, value: i64) {
        self.0.push(9);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn i32(&mut self, value: i32) {
        self.0.push(7);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn f32_bits(&mut self, value: u32) {
        self.0.push(13);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
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
            if records.len() > MAX_TASK_RESULTS {
                return Err(LobbyError("too many task results"));
            }
            out.u32(0);
            out.u8(task);
            out.u32(records.len() as u32);
            if !records.is_empty() {
                out.u32(records.len() as u32);
                for record in records {
                    if out.0.len() + record.len()
                        > crate::lobby::channel::MAX_SERVER_APPLICATION_BODY_BYTES
                    {
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
    // Empty input cannot exceed the record bound.
    player_data(user_id, &[]).expect("empty player data")
}

/// Full inventory records inherit their account from the enclosing player data
/// (VA 0xa4b850). Unspecified fields use target constructor 0xa4b4e0 defaults.
pub fn player_data(user_id: u64, items: &[u32]) -> Result<Vec<u8>, LobbyError> {
    player_data_with_balances(user_id, items, &[])
}

/// Currency decoder VA 0xa64ad0 inherits the enclosing account (except -1),
/// then reads UChar currency and UInt32 amount. Target holds at most 10.
pub fn player_data_with_balances(
    user_id: u64,
    items: &[u32],
    balances: &[(u8, u32)],
) -> Result<Vec<u8>, LobbyError> {
    let inventory: Vec<_> = items
        .iter()
        .map(|&id| InventoryItem { id, xp: 0 })
        .collect();
    player_data_with_inventory(user_id, &inventory, balances)
}

/// Permanent owned item. XP is cumulative weapon XP, consumed through
/// 0x695bc0 -> 0x695a80 (owned record +0x18) -> 0x46eb20.
/// Item IDs are marketplace IDs, not registered weapon indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventoryItem {
    pub id: u32,
    pub xp: u32,
}

/// Prepare the same player-data response with explicit per-item XP.
/// This serializer neither grants items nor decides which XP is authoritative.
pub fn player_data_with_inventory(
    user_id: u64,
    items: &[InventoryItem],
    balances: &[(u8, u32)],
) -> Result<Vec<u8>, LobbyError> {
    let mut currencies = std::collections::HashSet::new();
    if balances.len() > 10
        || balances
            .iter()
            .any(|(id, _)| *id == 0 || !currencies.insert(*id))
    {
        return Err(LobbyError("invalid local currency entries"));
    }
    validate_inventory_progress(items)?;
    let mut out = Writer::default();
    out.u64(user_id);
    out.string("");
    out.u32(balances.len() as u32);
    for (id, amount) in balances {
        if user_id == u64::MAX {
            out.u64(user_id);
            out.string("");
        }
        out.u8(*id);
        out.u32(*amount);
    }
    out.u32(items.len() as u32);
    for item in items {
        if user_id == u64::MAX {
            out.u64(user_id);
            out.string("");
        }
        inventory_fields(&mut out, *item)?;
    }
    if user_id == u64::MAX {
        // bdUserAccountID(-1, "") means no inherited identity to the nested reader.
        out.u64(user_id);
        out.string("");
    }
    out.u32(0); // Entitlement pair count is UInt32 on wire, UInt16 in memory.
    Ok(out.0)
}

fn validate_inventory(items: &[u32]) -> Result<(), LobbyError> {
    if items.len() > MAX_INVENTORY_ITEMS || items.contains(&0) {
        return Err(LobbyError("invalid local inventory count/item id"));
    }
    let mut seen = std::collections::HashSet::new();
    if items.iter().any(|id| !seen.insert(*id)) {
        return Err(LobbyError("duplicate local item id"));
    }
    Ok(())
}
fn validate_inventory_progress(items: &[InventoryItem]) -> Result<(), LobbyError> {
    validate_inventory(&items.iter().map(|item| item.id).collect::<Vec<_>>())?;
    if items.iter().any(|item| item.xp > i32::MAX as u32) {
        return Err(LobbyError("weapon XP exceeds native signed lookup range"));
    }
    Ok(())
}
fn inventory_fields(out: &mut Writer, item: InventoryItem) -> Result<(), LobbyError> {
    out.u32(item.id);
    out.u32(1);
    out.u32(item.xp);
    out.blob(&[])?;
    out.u32(u32::MAX);
    out.i64(i64::MAX);
    out.u16(0);
    out.u32(u32::MAX);
    out.u8(0);
    Ok(())
}

/// 80/203 binds standalone inventory entries (VA 0xa468c0/0x4a6a9f).
/// The default account passed by 0xa4b790 requires an explicit owner per entry.
pub fn inventory_records(user_id: u64, items: &[u32]) -> Result<Vec<Vec<u8>>, LobbyError> {
    let inventory: Vec<_> = items
        .iter()
        .map(|&id| InventoryItem { id, xp: 0 })
        .collect();
    inventory_records_with_progress(user_id, &inventory)
}

/// 80/203 refresh must carry the same XP as the initial player-data response.
pub fn inventory_records_with_progress(
    user_id: u64,
    items: &[InventoryItem],
) -> Result<Vec<Vec<u8>>, LobbyError> {
    validate_inventory_progress(items)?;
    items
        .iter()
        .map(|item| {
            let mut out = Writer::default();
            out.u64(user_id);
            out.string("");
            inventory_fields(&mut out, *item)?;
            Ok(out.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_xp_reaches_both_response_forms_and_preserves_legacy_zero_bytes() {
        let items = [
            InventoryItem {
                id: 10001,
                xp: 121_230,
            },
            InventoryItem { id: 10049, xp: 500 },
        ];
        for user in [123, u64::MAX] {
            let wire = player_data_with_inventory(user, &items, &[]).unwrap();
            let mut reader = Reader::new(&wire);
            assert_eq!(reader.u64().unwrap(), user);
            assert_eq!(reader.string(16).unwrap(), "");
            assert_eq!(reader.u32().unwrap(), 0);
            assert_eq!(reader.u32().unwrap(), 2);
            for item in items {
                if user == u64::MAX {
                    assert_eq!(reader.u64().unwrap(), user);
                    assert_eq!(reader.string(16).unwrap(), "");
                }
                assert_eq!(reader.u32().unwrap(), item.id);
                assert_eq!(reader.u32().unwrap(), 1);
                assert_eq!(reader.u32().unwrap(), item.xp);
                assert!(reader.blob().unwrap().is_empty());
                assert_eq!(reader.u32().unwrap(), u32::MAX);
                assert_eq!(
                    reader.take(9).unwrap(),
                    &[9, 255, 255, 255, 255, 255, 255, 255, 127]
                );
                assert_eq!(reader.u16().unwrap(), 0);
                assert_eq!(reader.u32().unwrap(), u32::MAX);
                assert_eq!(reader.u8().unwrap(), 0);
            }
            if user == u64::MAX {
                assert_eq!(reader.u64().unwrap(), user);
                assert_eq!(reader.string(16).unwrap(), "");
            }
            assert_eq!(reader.u32().unwrap(), 0);
            assert!(reader.remaining().is_empty());
            let records = inventory_records_with_progress(user, &items).unwrap();
            for (record, item) in records.iter().zip(items) {
                let mut reader = Reader::new(record);
                assert_eq!(reader.u64().unwrap(), user);
                assert_eq!(reader.string(16).unwrap(), "");
                assert_eq!(reader.u32().unwrap(), item.id);
                assert_eq!(reader.u32().unwrap(), 1);
                assert_eq!(reader.u32().unwrap(), item.xp);
            }
            let zero = items.map(|item| InventoryItem { xp: 0, ..item });
            assert_eq!(
                player_data_with_inventory(user, &zero, &[(1, 12)]).unwrap(),
                player_data_with_balances(user, &[10001, 10049], &[(1, 12)]).unwrap()
            );
            assert_eq!(
                inventory_records_with_progress(user, &zero).unwrap(),
                inventory_records(user, &[10001, 10049]).unwrap()
            );
        }
        for invalid in [
            vec![InventoryItem { id: 0, xp: 1 }],
            vec![items[0]; 2],
            vec![InventoryItem {
                id: 10001,
                xp: u32::MAX,
            }],
        ] {
            assert!(player_data_with_inventory(123, &invalid, &[]).is_err());
            assert!(inventory_records_with_progress(123, &invalid).is_err());
        }
    }
    #[test]
    fn currency_records_match_target_tags_and_inherit_owner() {
        let wire = player_data_with_balances(1, &[], &[(1, 100_000), (2, 1_000_000)]).unwrap();
        assert_eq!(
            &wire[11..],
            &[
                8, 2, 0, 0, 0, 3, 1, 8, 160, 134, 1, 0, 3, 2, 8, 64, 66, 15, 0, 8, 0, 0, 0, 0, 8,
                0, 0, 0, 0
            ]
        );
        let sentinel = player_data_with_balances(u64::MAX, &[], &[(1, 0)]).unwrap();
        assert_eq!(&sentinel[16..27], &sentinel[..11]);
        assert_eq!(&sentinel[27..34], &[3, 1, 8, 0, 0, 0, 0]);
        assert!(player_data_with_balances(1, &[], &[(1, 0), (1, 2)]).is_err());
        assert!(player_data_with_balances(1, &[], &[(0, 1)]).is_err());
        assert!(
            player_data_with_balances(1, &[], &(1..=11).map(|id| (id, 0)).collect::<Vec<_>>())
                .is_err()
        );
    }
    #[test]
    fn inventory_record_matches_independent_target_field_vector() {
        let v: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/player-inventory-synthetic.json"
        )))
        .unwrap();
        let wire = v["record"].as_str().unwrap();
        let expected: Vec<u8> = (0..wire.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&wire[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(
            player_data(0x8000_0000_0000_0002, &[792, 10001]).unwrap(),
            expected
        );
        assert!(player_data(1, &[792, 792]).is_err());
        assert!(player_data(1, &[0]).is_err());
        assert!(player_data(1, &(1..=MAX_INVENTORY_ITEMS as u32 + 1).collect::<Vec<_>>()).is_err());
    }
    #[test]
    fn complete_inventory_and_refresh_fit_the_advertised_target_frame() {
        let items: Vec<_> = (1..=MAX_INVENTORY_ITEMS as u32).collect();
        for user in [1, u64::MAX] {
            let inventory = player_data(user, &items).unwrap();
            let mut reader = Reader::new(&inventory);
            assert_eq!(reader.u64().unwrap(), user);
            assert_eq!(reader.string(16).unwrap(), "");
            assert_eq!(reader.u32().unwrap(), 0); // currencies
            assert_eq!(reader.u32().unwrap(), MAX_INVENTORY_ITEMS as u32);
            let full = reply(1, 230, Ok(&[inventory])).unwrap();
            let entries = inventory_records(user, &items).unwrap();
            assert_eq!(entries.len(), items.len());
            let mut last = Reader::new(entries.last().unwrap());
            assert_eq!(last.u64().unwrap(), user);
            assert_eq!(last.string(16).unwrap(), "");
            assert_eq!(last.u32().unwrap(), MAX_INVENTORY_ITEMS as u32);
            for body in [full, reply(2, 203, Ok(&entries)).unwrap()] {
                assert!(body.len() > 65536);
                let wire = crate::lobby::channel::encrypt_application(
                    1,
                    &body,
                    &[1; 72],
                    crate::lobby::channel::Direction::ServerToClient,
                    1,
                    [2; 16],
                )
                .unwrap();
                assert!(wire.len() - 4 <= crate::lobby::channel::MAX_SERVER_FRAME_BYTES);
                let decoded = crate::lobby::channel::decrypt_application(
                    &wire,
                    &[1; 72],
                    crate::lobby::channel::Direction::ServerToClient,
                    1,
                )
                .unwrap();
                assert_eq!(decoded.body, body);
            }
        }
    }
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
            crate::lobby::channel::registration_status(1)
        );
        assert!(reply(1, 4, Err(0)).is_err());
        assert!(
            reply(
                1,
                4,
                Ok(&[vec![
                    0;
                    crate::lobby::channel::MAX_SERVER_APPLICATION_BODY_BYTES
                ]])
            )
            .is_err()
        );
    }
}
