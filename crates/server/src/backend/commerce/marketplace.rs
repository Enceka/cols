//! Local permanent-item purchases. Funds and grants share one atomic wallet file.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Payment {
    Cp,
    Gp,
}
impl Payment {
    /// Confirmed against this build's gold GP / blue CP icons and live purchases.
    /// The native GetCP/GetGP labels are misleading for the shipped UI.
    pub fn wire_currency(self) -> u8 {
        match self {
            Self::Gp => 1,
            Self::Cp => 2,
        }
    }
}
impl<'de> Deserialize<'de> for Payment {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Name(String),
            Legacy(u8),
        }
        match Input::deserialize(deserializer)? {
            Input::Name(s) if s == "cp" => Ok(Self::Cp),
            Input::Name(s) if s == "gp" => Ok(Self::Gp),
            // Read the first local experiment's payment labels without rewriting
            // its history as wire currency IDs or deleting already purchased items.
            Input::Legacy(1) => Ok(Self::Cp),
            Input::Legacy(2) => Ok(Self::Gp),
            _ => Err(serde::de::Error::custom("unsupported payment kind")),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grant {
    pub sku_id: u32,
    pub item_id: u32,
    #[serde(alias = "currency")]
    pub payment: Payment,
    pub price: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalWallet {
    pub schema_version: u32,
    pub client_sha256: String,
    pub title_id: u32,
    pub user_id: String,
    pub cp: u32,
    pub gp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub purchases: Vec<Grant>,
}
impl LocalWallet {
    pub fn validate(&self, title_id: u32, user_id: u64) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.client_sha256 == codol_protocol::CLIENT_SHA256
                && self.title_id == title_id
                && self.user_id.parse::<u64>()? == user_id,
            "wallet identity/version mismatch"
        );
        let mut owned = HashSet::new();
        ensure!(
            self.purchases.len() <= 128
                && self
                    .purchases
                    .iter()
                    .all(|p| valid_offer(p) && owned.insert(p.item_id)),
            "invalid persisted purchases"
        );
        Ok(())
    }

    pub fn purchase(
        &mut self,
        shop: &LocalShop,
        inventory: &[u32],
        sku: u32,
        payment: Payment,
    ) -> Result<()> {
        let offer = shop
            .offers
            .iter()
            .find(|p| p.sku_id == sku && p.payment == payment)
            .ok_or_else(|| anyhow::anyhow!("SKU not in verified local permanent-item shop"))?;
        let mut owned: HashSet<_> = inventory.iter().copied().collect();
        owned.extend(self.purchases.iter().map(|p| p.item_id));
        ensure!(
            !owned.contains(&offer.item_id),
            "permanent item already owned"
        );
        ensure!(owned.len() < 128, "inventory limit reached");
        let balance = if payment == Payment::Cp {
            &mut self.cp
        } else {
            &mut self.gp
        };
        ensure!(*balance >= offer.price, "insufficient local funds");
        *balance -= offer.price;
        self.purchases.push(offer.clone());
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalShop {
    pub schema_version: u32,
    pub client_sha256: String,
    pub title_id: u32,
    pub source_sha256: String,
    pub offers: Vec<Grant>,
}
fn valid_offer(p: &Grant) -> bool {
    (1..=65535).contains(&p.sku_id)
        && (1..=65535).contains(&p.item_id)
        && (1..=65535).contains(&p.price)
}
impl LocalShop {
    pub fn validate(&self, title_id: u32) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.title_id == title_id
                && self.client_sha256 == codol_protocol::CLIENT_SHA256
                && self.source_sha256.len() == 64
                && self.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "shop identity/version mismatch"
        );
        let mut skus = HashSet::new();
        ensure!(
            !self.offers.is_empty()
                && self.offers.len() <= 30000
                && self
                    .offers
                    .iter()
                    .all(|p| valid_offer(p) && skus.insert(p.sku_id)),
            "invalid local shop offers"
        );
        Ok(())
    }
}
