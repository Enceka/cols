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
const MAX_ARCHIVES: usize = 4096;
const MAX_RETIRED: usize = 4096;

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

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedReceipt {
    award_sha256: String,
    receipt: Receipt,
    acknowledged: bool,
}
/// Cumulative totals behind the live receipt window. Revision zero keeps every
/// receipt in the window and starts from the imported private profile.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Base {
    revision: u64,
    player_total: u32,
    weapon_xp: BTreeMap<u32, u32>,
}
/// One immutable segment file released from the live receipt window.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Archived {
    file: String,
    from_revision: u64,
    to_revision: u64,
    sha256: String,
}
/// Retry tombstone for a receipt that already left the live window. Only the
/// most recent tombstones are kept, because a retry follows delivery by seconds.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Retired {
    match_id: String,
    award_sha256: String,
    revision: u64,
}
/// Self-describing archive segment; replaying it reproduces the boundary totals.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    schema_version: u32,
    client_sha256: String,
    title_id: u32,
    user_id: String,
    seed_private_sha256: String,
    rules_sha256: String,
    from_revision: u64,
    to_revision: u64,
    player_total_before: u32,
    weapon_xp_before: BTreeMap<u32, u32>,
    player_total_after: u32,
    weapon_xp_after: BTreeMap<u32, u32>,
    receipts: Vec<SavedReceipt>,
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
    #[serde(default)]
    base: Base,
    #[serde(default)]
    archived: Vec<Archived>,
    #[serde(default)]
    retired: Vec<Retired>,
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
fn replay(
    tables: &Tables,
    receipts: &[SavedReceipt],
    first_revision: u64,
    player_total: &mut u32,
    weapon_totals: &mut BTreeMap<u32, u32>,
) -> Result<()> {
    for (index, saved) in receipts.iter().enumerate() {
        ensure!(
            match_id(&saved.receipt.match_id)
                && saved.receipt.revision == first_revision + index as u64
                && hex_digest(&saved.award_sha256),
            "invalid persisted progression receipt"
        );
        ensure!(
            saved.receipt.player
                == tables
                    .player
                    .award(*player_total, saved.receipt.player.credited_xp)?,
            "persisted player receipt disagrees with XP history"
        );
        *player_total = saved.receipt.player.after.total_xp;
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
    Ok(())
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
                && self.revision == self.base.revision + self.receipts.len() as u64
                && self.receipts.len() <= MAX_RECEIPTS
                && self.archived.len() <= MAX_ARCHIVES
                && self.retired.len() <= MAX_RETIRED
                && self.weapon_xp.len() <= 512,
            "progression ledger identity, import source or revision mismatch"
        );
        let mut expected = 1;
        for (index, archived) in self.archived.iter().enumerate() {
            ensure!(
                archived.file == format!("progression-archive-{index:04}.json")
                    && archived.from_revision == expected
                    && archived.to_revision >= archived.from_revision
                    && archived.to_revision - archived.from_revision < MAX_RECEIPTS as u64
                    && hex_digest(&archived.sha256),
                "invalid progression archive segment"
            );
            expected = archived.to_revision + 1;
        }
        ensure!(
            expected - 1 == self.base.revision
                && (self.base.revision > 0
                    || (self.base.player_total == 0 && self.base.weapon_xp.is_empty())),
            "progression archive boundary disagrees with its segments"
        );
        let private = STANDARD.decode(&self.private_base64)?;
        let tables = Tables::native()?;
        let private_total =
            tables.player_total(profile::player_progress(&private, account.user_id)?)?;
        let seed_total = tables.player_total(profile::player_progress(seed, account.user_id)?)?;
        ensure!(
            seed_total <= private_total,
            "progression ledger regresses imported player XP"
        );
        let weapon_xp_valid = |map: &BTreeMap<u32, u32>| {
            map.iter()
                .all(|(id, xp)| (1..=65535).contains(id) && *xp <= MAX_XP)
        };
        ensure!(
            weapon_xp_valid(&self.weapon_xp) && weapon_xp_valid(&self.base.weapon_xp),
            "invalid persisted weapon XP"
        );
        let mut ids = HashSet::new();
        for retired in &self.retired {
            ensure!(
                match_id(&retired.match_id)
                    && hex_digest(&retired.award_sha256)
                    && (1..=self.base.revision).contains(&retired.revision)
                    && ids.insert(&retired.match_id),
                "invalid retired progression receipt"
            );
        }
        for saved in &self.receipts {
            ensure!(
                ids.insert(&saved.receipt.match_id),
                "invalid persisted progression receipt"
            );
        }
        let mut player_total = if self.base.revision == 0 {
            seed_total
        } else {
            ensure!(
                self.base.player_total >= seed_total,
                "progression archive boundary regresses imported player XP"
            );
            self.base.player_total
        };
        let mut weapon_totals = self.base.weapon_xp.clone();
        replay(
            &tables,
            &self.receipts,
            self.base.revision + 1,
            &mut player_total,
            &mut weapon_totals,
        )?;
        let final_progress = tables.player.progress(player_total)?;
        ensure!(
            weapon_totals == self.weapon_xp
                && private
                    == profile::with_player_progress(
                        seed,
                        account.user_id,
                        final_progress.level,
                        final_progress.xp_within_level,
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
                base: Base::default(),
                archived: vec![],
                retired: vec![],
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
        if let Some(receipt) =
            retired_receipt(&directory, &ledger, &award.match_id, &award_sha256).await?
        {
            return Ok(Commit {
                duplicate: true,
                receipt,
            });
        }
        if ledger.receipts.len() >= MAX_RECEIPTS {
            release(&directory, &mut ledger, account, &seed).await?;
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
    /// Release the acknowledged prefix of the live window into one immutable
    /// segment so long-running accounts keep committing without losing history.
    pub(crate) async fn archive(&self, account: AccountId) -> Result<Option<Archived>> {
        let _lock = self.store.writes.lock().await;
        let directory = self
            .store
            .profile_directory(account.title_id, account.user_id)
            .await?;
        let path = directory.join("progression.json");
        let Some(bytes) = read_bounded(&path, MAX_LEDGER_BYTES).await? else {
            return Ok(None);
        };
        let seed = read_bounded(&self.store.profile_path(account, true).await?, 61440)
            .await?
            .context("progression import profile disappeared")?;
        let mut ledger: Ledger = serde_json::from_slice(&bytes)?;
        ledger.validate(account, &seed)?;
        let Some(archived) = release(&directory, &mut ledger, account, &seed).await? else {
            return Ok(None);
        };
        ledger.validate(account, &seed)?;
        let bytes = serde_json::to_vec(&ledger)?;
        ensure!(
            bytes.len() <= MAX_LEDGER_BYTES,
            "progression ledger exceeds byte budget"
        );
        write_ledger(&path, &bytes).await?;
        Ok(Some(archived))
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

/// Move the acknowledged prefix of the receipt window into one immutable
/// segment. Acknowledgement is the terminal delivery boundary, so released
/// receipts stay available for audit and retry detection.
async fn release(
    directory: &std::path::Path,
    ledger: &mut Ledger,
    account: AccountId,
    seed: &[u8],
) -> Result<Option<Archived>> {
    let count = ledger
        .receipts
        .iter()
        .take_while(|receipt| receipt.acknowledged)
        .count();
    if count == 0 {
        return Ok(None);
    }
    ensure!(
        ledger.archived.len() < MAX_ARCHIVES,
        "progression archive segment capacity reached"
    );
    let tables = Tables::native()?;
    let mut player_total = if ledger.base.revision == 0 {
        tables.player_total(profile::player_progress(seed, account.user_id)?)?
    } else {
        ledger.base.player_total
    };
    let mut weapon_totals = ledger.base.weapon_xp.clone();
    let player_total_before = player_total;
    let weapon_xp_before = weapon_totals.clone();
    let from_revision = ledger.base.revision + 1;
    let to_revision = ledger.base.revision + count as u64;
    replay(
        &tables,
        &ledger.receipts[..count],
        from_revision,
        &mut player_total,
        &mut weapon_totals,
    )?;
    let segment = Segment {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        title_id: account.title_id,
        user_id: account.user_id.to_string(),
        seed_private_sha256: ledger.seed_private_sha256.clone(),
        rules_sha256: ledger.rules_sha256.clone(),
        from_revision,
        to_revision,
        player_total_before,
        weapon_xp_before,
        player_total_after: player_total,
        weapon_xp_after: weapon_totals.clone(),
        receipts: ledger.receipts[..count].to_vec(),
    };
    let bytes = serde_json::to_vec(&segment)?;
    ensure!(
        bytes.len() <= MAX_LEDGER_BYTES,
        "progression archive segment exceeds byte budget"
    );
    let archived = Archived {
        file: format!("progression-archive-{:04}.json", ledger.archived.len()),
        from_revision,
        to_revision,
        sha256: digest(&bytes),
    };
    write_ledger(&directory.join(&archived.file), &bytes).await?;
    let written = read_bounded(&directory.join(&archived.file), MAX_LEDGER_BYTES)
        .await?
        .context("progression archive segment disappeared")?;
    ensure!(
        digest(&written) == archived.sha256,
        "progression archive segment digest mismatch"
    );
    for receipt in ledger.receipts.drain(..count) {
        ledger.retired.push(Retired {
            match_id: receipt.receipt.match_id,
            award_sha256: receipt.award_sha256,
            revision: receipt.receipt.revision,
        });
    }
    if ledger.retired.len() > MAX_RETIRED {
        let excess = ledger.retired.len() - MAX_RETIRED;
        ledger.retired.drain(..excess);
    }
    ledger.base = Base {
        revision: to_revision,
        player_total,
        weapon_xp: weapon_totals,
    };
    ledger.archived.push(archived.clone());
    Ok(Some(archived))
}

/// Resolve a retried match that already left the live window.
async fn retired_receipt(
    directory: &std::path::Path,
    ledger: &Ledger,
    match_id: &str,
    award_sha256: &str,
) -> Result<Option<Receipt>> {
    let Some(retired) = ledger
        .retired
        .iter()
        .find(|retired| retired.match_id == match_id)
    else {
        return Ok(None);
    };
    ensure!(
        retired.award_sha256 == award_sha256,
        "conflicting progression retry"
    );
    let archived = ledger
        .archived
        .iter()
        .find(|archived| {
            archived.from_revision <= retired.revision && retired.revision <= archived.to_revision
        })
        .context("retired progression receipt is outside the archive")?;
    let bytes = read_bounded(&directory.join(&archived.file), MAX_LEDGER_BYTES)
        .await?
        .context("progression archive segment disappeared")?;
    ensure!(
        digest(&bytes) == archived.sha256,
        "progression archive segment digest mismatch"
    );
    let segment: Segment = serde_json::from_slice(&bytes)?;
    let saved = segment
        .receipts
        .iter()
        .find(|saved| saved.receipt.revision == retired.revision)
        .context("progression archive segment lost the receipt")?;
    Ok(Some(saved.receipt.clone()))
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
