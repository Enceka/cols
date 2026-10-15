//! One authoritative file commits player progress, weapon XP and retry receipts
//! together. Legacy private.bin/inventory.json are immutable import sources.
use super::{Award, MAX_XP, Tables, UnlockRequirement};
use crate::contracts::account::AccountId;
use crate::contracts::progression::MatchAward;
#[cfg(test)]
use crate::contracts::progression::WeaponXp;
use crate::infra::storage::{accounts::AccountStore, atomic_private_write, read_bounded};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::{profile, task::InventoryItem};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

const MAX_LEDGER_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECEIPTS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponResult {
    pub item_id: u32,
    pub progress: Award,
    pub eligible_attachments: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub match_id: String,
    pub revision: u64,
    pub player: Award,
    pub weapons: Vec<WeaponResult>,
}

#[derive(Debug)]
pub struct Commit {
    pub duplicate: bool,
    pub receipt: Receipt,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    schema_version: u32,
    client_sha256: String,
    source_sha256: String,
    weapons: Vec<WeaponRule>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WeaponRule {
    item_id: u32,
    is_primary: bool,
    attachments: Vec<UnlockRequirement>,
}
impl Rules {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.client_sha256 == codol_protocol::CLIENT_SHA256
                && hex_digest(&self.source_sha256)
                && (1..=512).contains(&self.weapons.len()),
            "progression rules identity/version mismatch"
        );
        let mut ids = HashSet::new();
        for weapon in &self.weapons {
            ensure!(
                (1..=65535).contains(&weapon.item_id)
                    && ids.insert(weapon.item_id)
                    && weapon.attachments.len() <= 256,
                "invalid or duplicate progression weapon"
            );
            let maximum = if weapon.is_primary { 31 } else { 20 };
            let mut attachments = HashSet::new();
            ensure!(
                weapon
                    .attachments
                    .iter()
                    .all(|a| (1..=65535).contains(&a.item_id)
                        && (1..=maximum).contains(&a.displayed_level)
                        && attachments.insert(a.item_id)),
                "invalid or duplicate attachment eligibility rule"
            );
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedReceipt {
    award_sha256: String,
    receipt: Receipt,
    acknowledged: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: u32,
    client_sha256: String,
    title_id: u32,
    user_id: String,
    seed_private_sha256: String,
    rules_sha256: String,
    revision: u64,
    private_base64: String,
    weapon_xp: BTreeMap<u32, u32>,
    receipts: Vec<SavedReceipt>,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn match_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && value.bytes().any(|b| b != b'0')
}
impl Ledger {
    fn validate(&self, account: AccountId, seed: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            self.schema_version == 1
                && self.client_sha256 == codol_protocol::CLIENT_SHA256
                && self.title_id == account.title_id
                && self.user_id == account.user_id.to_string()
                && self.seed_private_sha256 == digest(seed)
                && hex_digest(&self.rules_sha256)
                && self.revision > 0
                && self.revision == self.receipts.len() as u64
                && self.receipts.len() <= MAX_RECEIPTS
                && self.weapon_xp.len() <= 512,
            "progression ledger identity, import source or revision mismatch"
        );
        let private = STANDARD.decode(&self.private_base64)?;
        let tables = Tables::native()?;
        tables.player_total(profile::player_progress(&private, account.user_id)?)?;
        let seed_progress = profile::player_progress(seed, account.user_id)?;
        ensure!(
            tables.player_total(seed_progress)?
                <= tables.player_total(profile::player_progress(&private, account.user_id)?)?,
            "progression ledger regresses imported player XP"
        );
        ensure!(
            self.weapon_xp
                .iter()
                .all(|(id, xp)| (1..=65535).contains(id) && *xp <= MAX_XP),
            "invalid persisted weapon XP"
        );
        let mut ids = HashSet::new();
        let mut player_total = tables.player_total(seed_progress)?;
        let mut weapon_totals = BTreeMap::new();
        for (index, saved) in self.receipts.iter().enumerate() {
            ensure!(
                match_id(&saved.receipt.match_id)
                    && ids.insert(&saved.receipt.match_id)
                    && saved.receipt.revision == index as u64 + 1
                    && hex_digest(&saved.award_sha256),
                "invalid persisted progression receipt"
            );
            ensure!(
                saved.receipt.player
                    == tables
                        .player
                        .award(player_total, saved.receipt.player.credited_xp)?,
                "persisted player receipt disagrees with XP history"
            );
            player_total = saved.receipt.player.after.total_xp;
            ensure!(
                saved.receipt.weapons.len() <= 120
                    && saved
                        .receipt
                        .weapons
                        .windows(2)
                        .all(|p| p[0].item_id < p[1].item_id),
                "persisted weapon receipts are not canonical"
            );
            for weapon in &saved.receipt.weapons {
                let before = *weapon_totals.get(&weapon.item_id).unwrap_or(&0);
                let primary = tables.primary.award(before, weapon.progress.credited_xp)?;
                let secondary = tables
                    .secondary
                    .award(before, weapon.progress.credited_xp)?;
                ensure!(
                    (1..=65535).contains(&weapon.item_id)
                        && (weapon.progress == primary || weapon.progress == secondary),
                    "persisted weapon receipt disagrees with XP history"
                );
                weapon_totals.insert(weapon.item_id, weapon.progress.after.total_xp);
            }
        }
        let final_progress = tables.player.progress(player_total)?;
        ensure!(
            weapon_totals == self.weapon_xp
                && private
                    == profile::with_player_progress(
                        seed,
                        account.user_id,
                        final_progress.level,
                        final_progress.xp_within_level
                    )?,
            "progression snapshot disagrees with committed receipts"
        );
        Ok(private)
    }
}

pub(crate) struct ProgressionService {
    store: Arc<AccountStore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    schema_version: u32,
    client_sha256: String,
    enabled: bool,
    awards: crate::contracts::progression::Policy,
}
impl ProgressionService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }
    pub(crate) async fn policy(&self) -> Result<Option<crate::contracts::progression::Policy>> {
        let Some(bytes) =
            read_bounded(&self.store.root.join("progression-policy.json"), 16384).await?
        else {
            return Ok(None);
        };
        let file: PolicyFile = serde_json::from_slice(&bytes)?;
        ensure!(
            file.schema_version == 1
                && file.client_sha256 == codol_protocol::CLIENT_SHA256
                && file.awards.valid(),
            "invalid local progression policy"
        );
        if !file.enabled {
            return Ok(None);
        }
        let bytes = read_bounded(&self.store.root.join("progression-rules.json"), 1024 * 1024)
            .await?
            .context("enabled progression needs verified weapon rules")?;
        let rules: Rules = serde_json::from_slice(&bytes)?;
        rules.validate()?;
        Ok(Some(file.awards))
    }
    /// A single rename is the commit point. Cancellation/failure before rename
    /// changes nothing; retry after rename finds the original receipt.
    pub(crate) async fn commit(&self, account: AccountId, award: MatchAward) -> Result<Commit> {
        ensure!(
            match_id(&award.match_id)
                && award.player_xp <= MAX_XP
                && award.weapons.len() <= 120
                && (award.player_xp > 0 || !award.weapons.is_empty()),
            "invalid match progression award"
        );
        ensure!(
            award
                .weapons
                .iter()
                .all(|w| w.item_id > 0 && w.xp > 0 && w.xp <= MAX_XP)
                && award
                    .weapons
                    .windows(2)
                    .all(|p| p[0].item_id < p[1].item_id),
            "weapon awards must have canonical unique item IDs"
        );
        let award_sha256 = digest(&serde_json::to_vec(&award)?);
        let _lock = self.store.writes.lock().await;
        let directory = self
            .store
            .profile_directory(account.title_id, account.user_id)
            .await?;
        let seed = read_bounded(&self.store.profile_path(account, true).await?, 61440)
            .await?
            .context("progression requires an existing private profile")?;
        let seed_progress = profile::player_progress(&seed, account.user_id)?;
        let tables = Tables::native()?;
        tables.player_total(seed_progress)?;
        let rules_bytes =
            read_bounded(&self.store.root.join("progression-rules.json"), 1024 * 1024)
                .await?
                .context("verified progression rules are missing")?;
        let rules: Rules = serde_json::from_slice(&rules_bytes)?;
        rules.validate()?;
        let rules_sha256 = digest(&rules_bytes);
        let path = directory.join("progression.json");
        let mut ledger = match read_bounded(&path, MAX_LEDGER_BYTES).await? {
            Some(bytes) => {
                let ledger: Ledger = serde_json::from_slice(&bytes)?;
                ledger.validate(account, &seed)?;
                ensure!(
                    ledger.rules_sha256 == rules_sha256,
                    "progression rules changed; explicit migration required"
                );
                ledger
            }
            None => Ledger {
                schema_version: 1,
                client_sha256: codol_protocol::CLIENT_SHA256.into(),
                title_id: account.title_id,
                user_id: account.user_id.to_string(),
                seed_private_sha256: digest(&seed),
                rules_sha256,
                revision: 0,
                private_base64: STANDARD.encode(&seed),
                weapon_xp: BTreeMap::new(),
                receipts: vec![],
            },
        };
        if let Some(saved) = ledger
            .receipts
            .iter()
            .find(|r| r.receipt.match_id == award.match_id)
        {
            ensure!(
                saved.award_sha256 == award_sha256,
                "conflicting progression retry"
            );
            return Ok(Commit {
                duplicate: true,
                receipt: saved.receipt.clone(),
            });
        }
        ensure!(
            ledger.receipts.len() < MAX_RECEIPTS,
            "progression receipt capacity reached; archive migration required"
        );
        // Ownership includes permanent purchases, without changing either file.
        let inventory = crate::backend::inventory::InventoryService::new(self.store.clone());
        let mut owned: HashSet<_> = inventory.items(account).await?.into_iter().collect();
        if let Some(wallet) =
            crate::backend::commerce::service::CommerceService::new(self.store.clone())
                .wallet(account)
                .await?
        {
            owned.extend(wallet.purchases.iter().map(|p| p.item_id));
        }
        let private = STANDARD.decode(&ledger.private_base64)?;
        let player = tables.plan_player_award(
            profile::player_progress(&private, account.user_id)?,
            award.player_xp,
        )?;
        let mut weapons = Vec::new();
        for grant in &award.weapons {
            ensure!(
                owned.contains(&grant.item_id),
                "weapon XP cannot grant an unowned item"
            );
            let rule = rules
                .weapons
                .iter()
                .find(|w| w.item_id == grant.item_id)
                .context("weapon has no verified progression classification")?;
            let curve = if rule.is_primary {
                &tables.primary
            } else {
                &tables.secondary
            };
            let progress = curve.award(
                *ledger.weapon_xp.get(&grant.item_id).unwrap_or(&0),
                grant.xp,
            )?;
            let eligible_attachments = progress.crossed_unlocks(&rule.attachments);
            ledger
                .weapon_xp
                .insert(grant.item_id, progress.after.total_xp);
            weapons.push(WeaponResult {
                item_id: grant.item_id,
                progress,
                eligible_attachments,
            });
        }
        let updated_private = profile::with_player_progress(
            &private,
            account.user_id,
            player.after.level,
            player.after.xp_within_level,
        )?;
        ledger.private_base64 = STANDARD.encode(updated_private);
        ledger.revision += 1;
        let receipt = Receipt {
            match_id: award.match_id,
            revision: ledger.revision,
            player,
            weapons,
        };
        ledger.receipts.push(SavedReceipt {
            award_sha256,
            receipt: receipt.clone(),
            acknowledged: false,
        });
        ledger.validate(account, &seed)?;
        let bytes = serde_json::to_vec(&ledger)?;
        ensure!(
            bytes.len() <= MAX_LEDGER_BYTES,
            "progression ledger exceeds byte budget"
        );
        write_ledger(&path, &bytes).await?;
        Ok(Commit {
            duplicate: false,
            receipt,
        })
    }
    pub(crate) async fn pending(&self, account: AccountId) -> Result<Vec<Receipt>> {
        Ok(load(&self.store, account)
            .await?
            .map_or_else(Vec::new, |(ledger, _)| {
                ledger
                    .receipts
                    .into_iter()
                    .filter(|r| !r.acknowledged)
                    .map(|r| r.receipt)
                    .collect()
            }))
    }
    /// Call after the actual delivery/consumption boundary has been confirmed.
    /// A HUD enqueue by itself is not an acknowledgement.
    pub(crate) async fn acknowledge(
        &self,
        account: AccountId,
        id: &str,
        revision: u64,
    ) -> Result<()> {
        let _lock = self.store.writes.lock().await;
        let (mut ledger, _) = load(&self.store, account)
            .await?
            .context("progression ledger missing")?;
        let saved = ledger
            .receipts
            .iter_mut()
            .find(|r| r.receipt.match_id == id && r.receipt.revision == revision)
            .context("progression receipt acknowledgement mismatch")?;
        if saved.acknowledged {
            return Ok(());
        }
        saved.acknowledged = true;
        let path = self
            .store
            .profile_directory(account.title_id, account.user_id)
            .await?
            .join("progression.json");
        write_ledger(&path, &serde_json::to_vec(&ledger)?).await
    }
}

async fn write_ledger(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    atomic_private_write(path, bytes).await?;
    #[cfg(unix)]
    tokio::fs::File::open(path.parent().context("progression directory missing")?)
        .await?
        .sync_all()
        .await?;
    Ok(())
}

async fn load(store: &AccountStore, account: AccountId) -> Result<Option<(Ledger, Vec<u8>)>> {
    let directory = store
        .profile_directory(account.title_id, account.user_id)
        .await?;
    let Some(bytes) = read_bounded(&directory.join("progression.json"), MAX_LEDGER_BYTES).await?
    else {
        return Ok(None);
    };
    let seed = read_bounded(&store.profile_path(account, true).await?, 61440)
        .await?
        .context("progression import profile disappeared")?;
    let ledger: Ledger = serde_json::from_slice(&bytes)?;
    let private = ledger.validate(account, &seed)?;
    Ok(Some((ledger, private)))
}

/// Shared projection used by profile RPCs and gameplay admission.
pub(crate) async fn committed_private(
    store: &AccountStore,
    account: AccountId,
) -> Result<Option<Vec<u8>>> {
    Ok(load(store, account).await?.map(|(_, private)| private))
}
/// Initial and refresh inventory replies share this projection. Missing records
/// keep the legacy XP=0 bytes; saved XP never grants an item absent from ownership.
pub(crate) async fn inventory_progress(
    store: &AccountStore,
    account: AccountId,
    owned: &[u32],
) -> Result<Vec<InventoryItem>> {
    let state = load(store, account).await?;
    Ok(owned
        .iter()
        .map(|&id| InventoryItem {
            id,
            xp: state
                .as_ref()
                .and_then(|(l, _)| l.weapon_xp.get(&id))
                .copied()
                .unwrap_or(0),
        })
        .collect())
}

#[cfg(test)]
mod tests;
