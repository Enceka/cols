//! Target single-SKU, self-purchase forms: 80/154 and 93/131.
//! Serializers 0xa45b40/0xa64ca0; UI wrappers request zero receipt records.
use crate::lobby::channel::LobbyError;
use crate::lobby::task::Reader;

pub fn single_sku(service: u8, arguments: &[u8]) -> Result<u32, LobbyError> {
    if service != 80 && service != 93 {
        return Err(LobbyError("unsupported purchase service"));
    }
    let mut r = Reader::new(arguments);
    if !r.string(16)?.is_empty() || r.u32()? != 1 {
        return Err(LobbyError("only one local SKU is supported"));
    }
    let sku = r.u32()?;
    if sku == 0 || sku > u16::MAX as u32 || r.u32()? != 1 || r.u32()? != 1 || r.boolean()? {
        return Err(LobbyError("unsupported purchase quantity/mode"));
    }
    // One default recipient (0, "") means the authenticated account.
    // Gifts, metadata, receipts and per-SKU overrides are separate work.
    if r.u32()? != 1
        || r.u64()? != 0
        || !r.string(16)?.is_empty()
        || r.u32()? != 0
        || r.u32()? != 0
        || r.u8()? != 0
        || r.u32()? != u32::from(service == 80)
    {
        return Err(LobbyError("unsupported purchase recipient/options"));
    }
    r.finish()?;
    Ok(sku)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_single_purchase_forms_reject_truncation_and_gifts() {
        let cp = [
            16, 0, 8, 1, 0, 0, 0, 8, 37, 57, 0, 0, 8, 1, 0, 0, 0, 8, 1, 0, 0, 0, 1, 0, 8, 1, 0, 0,
            0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 8, 0, 0, 0, 0, 8, 0, 0, 0, 0, 3, 0, 8, 0, 0, 0,
            0, 0,
        ];
        assert_eq!(single_sku(93, &cp).unwrap(), 14629);
        let mut gp = cp;
        gp[8..12].copy_from_slice(&10073u32.to_le_bytes());
        gp[53] = 1;
        assert_eq!(single_sku(80, &gp).unwrap(), 10073);
        for data in [&cp[..], &gp[..]] {
            let svc = if data[53] == 0 { 93 } else { 80 };
            for n in 0..data.len() {
                assert!(single_sku(svc, &data[..n]).is_err());
            }
            for offset in [3, 13, 18, 23, 25, 30, 41, 46, 51] {
                let mut bad = data.to_vec();
                bad[offset] = 2;
                assert!(single_sku(svc, &bad).is_err());
            }
            let mut extra = data.to_vec();
            extra.push(0);
            assert!(single_sku(svc, &extra).is_err());
        }
    }
}
