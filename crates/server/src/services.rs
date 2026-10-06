//! Opt-in local services. Opaque user files are isolated by authenticated
//! title/UID; no old-build profile or catalogue data is manufactured.
use crate::{auth_server::write_private, session::Session};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::{
    lobby, profile,
    task::{self, Request, StorageRequest, Writer},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tokio::{io::AsyncReadExt, sync::Mutex};

pub struct LocalServices {
    root: PathBuf,
    writes: Mutex<()>,
}
pub struct TaskResult {
    pub label: &'static str,
    pub result: Result<Vec<Vec<u8>>, u32>,
}
impl TaskResult {
    fn success(label: &'static str, records: Vec<Vec<u8>>) -> Self {
        Self {
            label,
            result: Ok(records),
        }
    }
    fn failure(label: &'static str, code: u32) -> Self {
        Self {
            label,
            result: Err(code),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFile {
    schema: u32,
    created: u32,
    modified: u32,
    visibility: bool,
    data_base64: String,
}

impl LocalServices {
    pub async fn open(root: PathBuf) -> Result<Self> {
        create_directory(&root).await?;
        create_directory(&root.join("publisher")).await?;
        create_directory(&root.join("users")).await?;
        create_directory(&root.join("profiles")).await?;
        Ok(Self {
            root: tokio::fs::canonicalize(root).await?,
            writes: Mutex::new(()),
        })
    }

    pub async fn handle(&self, body: &[u8], session: &Session, now: u32) -> Result<TaskResult> {
        let request = Request::parse(body)?;
        match (request.service, request.task) {
            (71, 4) => {
                lobby::registration_build(body)?;
                Ok(TaskResult::success("registration", vec![]))
            }
            (12, 6) => {
                task::Reader::new(request.arguments).finish()?;
                let mut record = Writer::default();
                record.u32(now);
                Ok(TaskResult::success("server_time", vec![record.0]))
            }
            (10, 10 | 12 | 21) => self.storage(task::storage(&request)?, session, now).await,
            (10, 20) => self.list_publisher(request.arguments).await,
            (80, 70 | 107) => {
                let mut reader = task::Reader::new(request.arguments);
                let context = reader.string(16)?;
                let page = reader.u32()?;
                let page_size = reader.u32()?;
                if request.task == 70 {
                    let _show_all = reader.boolean()?;
                    let sku_filters = reader.u32()?;
                    if sku_filters != 0 {
                        return Ok(TaskResult::failure(
                            "unsupported_catalogue_filter",
                            task::TASK_FAILED,
                        ));
                    }
                    let type_filters = reader.u32()?;
                    if type_filters != 0 {
                        return Ok(TaskResult::failure(
                            "unsupported_catalogue_filter",
                            task::TASK_FAILED,
                        ));
                    }
                }
                let _client_version = reader.string(32)?;
                reader.finish()?;
                if !context.is_empty() || page != 1 || !(1..=2000).contains(&page_size) {
                    return Ok(TaskResult::failure(
                        "unsupported_catalogue_query",
                        task::TASK_FAILED,
                    ));
                }
                // VA 0x4a6150 stops pagination when resultCount < pageSize,
                // including an empty first page. The lab has no saleable SKUs;
                // owned inventory and starter grants remain separate work.
                Ok(TaskResult::success(
                    if request.task == 70 {
                        "local_empty_catalogue"
                    } else {
                        "local_empty_catalogue_links"
                    },
                    vec![],
                ))
            }
            (80, 230) => {
                let mut reader = task::Reader::new(request.arguments);
                let context = reader.string(16)?;
                reader.finish()?;
                if !context.is_empty() {
                    return Ok(TaskResult::failure(
                        "unsupported_marketplace_namespace",
                        task::TASK_FAILED,
                    ));
                }
                Ok(TaskResult::success(
                    "local_empty_player_data",
                    vec![task::empty_player_data(session.user_id)],
                ))
            }
            (80, 173) => {
                let mut reader = task::Reader::new(request.arguments);
                let context = reader.string(16)?;
                let maximum = reader.u16()?;
                reader.finish()?;
                if !context.is_empty() || !(1..=700).contains(&maximum) {
                    return Ok(TaskResult::failure(
                        "unsupported_coupon_query",
                        task::TASK_FAILED,
                    ));
                }
                // VA 0xa46fb0 binds an array; VA 0x4a3470 consumes its count.
                // This local account has no issued coupons/mail attachments.
                Ok(TaskResult::success("empty_coupon_inventory", vec![]))
            }
            (93, 1) => {
                let mut reader = task::Reader::new(request.arguments);
                let maximum = reader.u32()?;
                reader.finish()?;
                if maximum != 1 {
                    return Ok(TaskResult::failure(
                        "unsupported_balance_query",
                        task::TASK_FAILED,
                    ));
                }
                // VA 0xa4c4a0 reads currency UChar + UInt32 amount. The game
                // checks currency 1 at VA 0x57d548. No local funds were granted.
                let mut record = Writer::default();
                record.u8(1);
                record.u32(0);
                Ok(TaskResult::success("local_zero_balance", vec![record.0]))
            }
            (8, 1 | 2) => {
                let mut reader = task::Reader::new(request.arguments);
                if request.task == 1 && reader.u64()? != session.user_id {
                    return Ok(TaskResult::failure(
                        "profile_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                }
                reader.finish()?;
                if request.task == 1 {
                    let path = self.profile_path(session).await?;
                    if let Some(data) = read_bounded(&path, task::MAX_FILE_BYTES).await? {
                        return Ok(TaskResult::success(
                            "public_profile",
                            vec![profile::public_record(session.user_id, &data)?],
                        ));
                    }
                }
                // Private profile writes are not supported yet. A precise
                // missing result lets the target initialize its own defaults.
                Ok(TaskResult::failure("profile_missing", task::NO_PROFILE))
            }
            (8, 3) => {
                profile::validate_public(request.arguments)?;
                let _lock = self.writes.lock().await;
                let path = self.profile_path(session).await?;
                atomic_private_write(&path, request.arguments).await?;
                Ok(TaskResult::success("public_profile_saved", vec![]))
            }
            _ => Ok(TaskResult::failure("unsupported_task", task::TASK_FAILED)),
        }
    }

    async fn profile_path(&self, session: &Session) -> Result<PathBuf> {
        let title = self
            .root
            .join("profiles")
            .join(session.title_id.to_string());
        create_directory(&title).await?;
        let user = title.join(session.user_id.to_string());
        create_directory(&user).await?;
        let path = user.join("public.bin");
        if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await {
            ensure!(
                metadata.is_file() && !metadata.is_symlink(),
                "profile path must be a regular file"
            );
        }
        Ok(path)
    }

    async fn list_publisher(&self, arguments: &[u8]) -> Result<TaskResult> {
        let mut reader = task::Reader::new(arguments);
        let context = reader.string(16)?;
        let filter = reader.u32()?;
        let limit = reader.u16()?;
        let offset = reader.u16()?;
        reader.finish()?;
        // Only the observed unfiltered query is supported; filter/offset
        // semantics need more evidence before implementing pagination.
        if !context.is_empty() || filter != 0 || offset != 0 || limit != 32 {
            return Ok(TaskResult::failure(
                "unsupported_publisher_query",
                task::TASK_FAILED,
            ));
        }
        let mut entries = tokio::fs::read_dir(self.root.join("publisher")).await?;
        let mut names = Vec::new();
        let mut scanned = 0;
        while let Some(entry) = entries.next_entry().await? {
            scanned += 1;
            ensure!(scanned <= 128, "publisher directory exceeds local limit");
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 publisher name"))?;
            if valid_name(&name) && (name.ends_with(".info") || name.ends_with(".ff")) {
                names.push(name);
            }
        }
        names.sort();
        ensure!(
            names.len() <= limit as usize,
            "publisher pagination is not implemented"
        );
        let mut records = Vec::new();
        for name in names.into_iter().take(limit as usize) {
            let path = self.root.join("publisher").join(&name);
            let Some(data) = read_bounded(&path, task::MAX_FILE_BYTES).await? else {
                continue;
            };
            let metadata = tokio::fs::metadata(&path).await?;
            let modified: u32 = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
                .try_into()?;
            ensure!(
                modified != 0 && modified != u32::MAX,
                "invalid publisher modification time"
            );
            let digest = Sha256::digest(name.as_bytes());
            let file_id = u64::from_le_bytes(digest[..8].try_into()?).max(1);
            // VA 0xa5b230: this result type differs from upload FileInfo.
            let mut record = Writer::default();
            record.u32(data.len() as u32);
            record.u64(file_id);
            record.u32(modified);
            record.u32(modified);
            record.boolean(true);
            record.u64(0);
            record.string(&name);
            record.string(""); // Required String(cap=50), locally unset; semantics still unknown.
            records.push(record.0);
        }
        Ok(TaskResult::success("publisher_file_list", records))
    }

    async fn storage(
        &self,
        request: StorageRequest<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let (context, name) = match &request {
            StorageRequest::Read { context, name, .. }
            | StorageRequest::Publisher { context, name }
            | StorageRequest::Upload { context, name, .. } => (*context, *name),
        };
        if !context.is_empty() || !valid_name(name) {
            return Ok(TaskResult::failure(
                "unsupported_storage_namespace",
                task::TASK_FAILED,
            ));
        }
        match request {
            StorageRequest::Publisher { .. } => {
                // Only operator-provided files, never generated empty config files.
                let path = self.root.join("publisher").join(name);
                match read_bounded(&path, task::MAX_FILE_BYTES).await? {
                    None => Ok(TaskResult::failure("publisher_file_missing", task::NO_FILE)),
                    Some(data) => blob_result("publisher_file", &data),
                }
            }
            StorageRequest::Read {
                owner,
                account_type,
                ..
            } => {
                if owner != session.user_id || !account_type.is_empty() {
                    return Ok(TaskResult::failure(
                        "storage_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                }
                let path = self.user_path(session, name).await?;
                match read_stored(&path).await? {
                    None => Ok(TaskResult::failure("user_file_missing", task::NO_FILE)),
                    Some(record) => blob_result("user_file", &STANDARD.decode(record.data_base64)?),
                }
            }
            StorageRequest::Upload {
                visibility,
                data,
                owner,
                account_type,
                version,
                ..
            } => {
                if (owner != 0 && owner != session.user_id)
                    || !account_type.is_empty()
                    || version != 0
                {
                    return Ok(TaskResult::failure(
                        "unsupported_upload_parameters",
                        task::TASK_FAILED,
                    ));
                }
                let _lock = self.writes.lock().await;
                let path = self.user_path(session, name).await?;
                let previous = read_stored(&path).await?;
                if previous.is_none() {
                    let mut entries =
                        tokio::fs::read_dir(path.parent().context("user folder")?).await?;
                    let mut count = 0;
                    while entries.next_entry().await?.is_some() {
                        count += 1;
                    }
                    ensure!(count < 128, "local user file quota exceeded");
                }
                let record = StoredFile {
                    schema: 1,
                    created: previous.map_or(now, |p| p.created),
                    modified: now,
                    visibility,
                    data_base64: STANDARD.encode(data),
                };
                let bytes = serde_json::to_vec(&record)?;
                atomic_private_write(&path, &bytes).await?;
                // Target bdFileInfo::deserialize VA 0xa4eab0. No fileID field.
                let mut info = Writer::default();
                info.u32(data.len() as u32);
                info.u32(record.created);
                info.u32(now);
                info.boolean(visibility);
                info.u64(session.user_id);
                info.string("");
                info.string(name);
                Ok(TaskResult::success("user_file_uploaded", vec![info.0]))
            }
        }
    }

    async fn user_path(&self, session: &Session, name: &str) -> Result<PathBuf> {
        let title = self.root.join("users").join(session.title_id.to_string());
        create_directory(&title).await?;
        let user = title.join(session.user_id.to_string());
        create_directory(&user).await?;
        Ok(user.join(format!("{name}.json")))
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
async fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut random = [0; 8];
    openssl::rand::rand_bytes(&mut random)?;
    let temporary = path.with_extension(format!("{:016x}.tmp", u64::from_le_bytes(random)));
    let result = async {
        write_private(&temporary, bytes).await?;
        tokio::fs::File::open(&temporary).await?.sync_all().await?;
        tokio::fs::rename(&temporary, path).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(temporary).await;
    }
    result
}
async fn create_directory(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        metadata.is_dir() && !metadata.is_symlink(),
        "local data directory must be a real directory"
    );
    Ok(())
}
async fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.is_symlink(),
        "local file must be a regular file"
    );
    ensure!(
        metadata.len() <= limit as u64,
        "local file exceeds byte limit"
    );
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= limit, "local file grew beyond byte limit");
    Ok(Some(bytes))
}
async fn read_stored(path: &Path) -> Result<Option<StoredFile>> {
    let Some(bytes) = read_bounded(path, task::MAX_FILE_BYTES * 2).await? else {
        return Ok(None);
    };
    let record: StoredFile = serde_json::from_slice(&bytes)?;
    ensure!(record.schema == 1, "unsupported local storage schema");
    ensure!(
        STANDARD.decode(&record.data_base64)?.len() <= task::MAX_FILE_BYTES,
        "stored blob exceeds byte limit"
    );
    Ok(Some(record))
}
fn blob_result(label: &'static str, data: &[u8]) -> Result<TaskResult> {
    let mut record = Writer::default();
    record.blob(data)?;
    Ok(TaskResult::success(label, vec![record.0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn initial_coupon_and_balance_queries_have_distinct_verified_result_shapes() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let coupons = [80, 3, 173, 16, 0, 6, 188, 2, 0];
        let balance = [93, 3, 1, 8, 1, 0, 0, 0, 0];
        assert_eq!(
            server.handle(&coupons, &s, 1).await.unwrap().result,
            Ok(vec![])
        );
        assert_eq!(
            server.handle(&balance, &s, 1).await.unwrap().result,
            Ok(vec![vec![3, 1, 8, 0, 0, 0, 0]])
        );
        for body in [&coupons[..], &balance[..]] {
            for size in 0..body.len() {
                assert!(server.handle(&body[..size], &s, 1).await.is_err());
            }
            let mut extra = body.to_vec();
            extra.push(0);
            assert!(server.handle(&extra, &s, 1).await.is_err());
        }
        let mut zero = coupons;
        zero[6] = 0;
        zero[7] = 0;
        assert_eq!(
            server.handle(&zero, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let mut other = balance;
        other[4] = 2;
        assert_eq!(
            server.handle(&other, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        assert_eq!(
            server.handle(&[80, 3, 172, 0], &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
    }
    fn session(uid: u64) -> Session {
        Session {
            title_id: 1002,
            user_id: uid,
            iv_seed: 1,
            expires_at: u32::MAX,
            key: [0; 24],
        }
    }
    fn read(name: &str, uid: u64) -> Vec<u8> {
        let mut w = Writer(vec![10, 3, 12]);
        w.string("");
        w.string(name);
        w.u64(uid);
        w.string("");
        w.0.push(0);
        w.0
    }
    fn upload(name: &str, data: &[u8]) -> Vec<u8> {
        let mut w = Writer(vec![10, 3, 10]);
        w.string("");
        w.string(name);
        w.boolean(true);
        w.blob(data).unwrap();
        w.u64(0);
        w.string("");
        w.u32(0);
        w.0.push(0);
        w.0
    }
    #[tokio::test]
    async fn upload_survives_restart_preserves_exact_bytes_and_isolates_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let s = session(u64::MAX);
        let server = LocalServices::open(root.clone()).await.unwrap();
        let request = read("the.json", s.user_id);
        assert_eq!(
            server.handle(&request, &s, 10).await.unwrap().result,
            Err(task::NO_FILE)
        );
        let data = b"{\"blacklist\":[],\"Recentmetlist\":[]}\0";
        let stored = server
            .handle(&upload("the.json", data), &s, 11)
            .await
            .unwrap()
            .result
            .unwrap();
        let mut expected = Writer::default();
        expected.u32(data.len() as u32);
        expected.u32(11);
        expected.u32(11);
        expected.boolean(true);
        expected.u64(s.user_id);
        expected.string("");
        expected.string("the.json");
        assert_eq!(stored, [expected.0]);
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let mut blob = Writer::default();
        blob.blob(data).unwrap();
        assert_eq!(
            server
                .handle(&request, &s, 12)
                .await
                .unwrap()
                .result
                .unwrap(),
            [blob.0]
        );
        assert_eq!(
            server
                .handle(&request, &session(1), 12)
                .await
                .unwrap()
                .result,
            Err(task::TASK_FAILED)
        );
        assert_eq!(
            server
                .handle(&read("the.json", 1), &session(1), 12)
                .await
                .unwrap()
                .result,
            Err(task::NO_FILE)
        );
        let mut different_title = s.clone();
        different_title.title_id = 1;
        assert_eq!(
            server
                .handle(&request, &different_title, 12)
                .await
                .unwrap()
                .result,
            Err(task::NO_FILE)
        );
        let updated = server
            .handle(&upload("the.json", b"new"), &s, 20)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(&updated[0][5..15], &[8, 11, 0, 0, 0, 8, 20, 0, 0, 0]);
    }
    #[tokio::test]
    async fn rejects_paths_corrupt_files_and_unsupported_tasks_without_success() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        for name in ["../secret", "/etc/passwd", "a\\b", ".hidden", ""] {
            assert_eq!(
                server
                    .handle(&upload(name, b"x"), &s, 1)
                    .await
                    .unwrap()
                    .result,
                Err(task::TASK_FAILED)
            );
        }
        assert_eq!(
            server.handle(&[80, 3, 69, 0], &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let path = server.user_path(&s, "the.json").await.unwrap();
        tokio::fs::write(&path, b"broken").await.unwrap();
        assert!(
            server
                .handle(&upload("the.json", b"x"), &s, 1)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"broken");
        let mut request = Writer(vec![10, 3, 21]);
        request.string("");
        request.string("configs_s.info");
        request.0.push(0);
        assert_eq!(
            server.handle(&request.0, &s, 1).await.unwrap().result,
            Err(task::NO_FILE)
        );
        tokio::fs::write(
            server.root.join("publisher/configs_s.info"),
            vec![0; task::MAX_FILE_BYTES + 1],
        )
        .await
        .unwrap();
        assert!(server.handle(&request.0, &s, 1).await.is_err());
    }

    #[tokio::test]
    async fn publisher_listing_matches_served_bytes_and_has_stable_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let encrypted = codol_protocol::publisher::crypt_configs(include_bytes!(
            "../../../configs/publisher/configs_s.ini"
        ))
        .unwrap();
        tokio::fs::write(server.root.join("publisher/configs_s.info"), &encrypted)
            .await
            .unwrap();
        let query = [10, 3, 20, 16, 0, 8, 0, 0, 0, 0, 6, 32, 0, 6, 0, 0, 0];
        let first = server.handle(&query, &s, 1).await.unwrap().result.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(
            first,
            server
                .handle(&query, &s, 999)
                .await
                .unwrap()
                .result
                .unwrap()
        );
        // Read in the exact order of target deserializer VA 0xa5b230.
        let mut record = first[0].clone();
        record.push(0);
        let mut r = task::Reader::new(&record);
        assert_eq!(r.u32().unwrap(), encrypted.len() as u32);
        assert_ne!(r.u64().unwrap(), 0);
        let created = r.u32().unwrap();
        let modified = r.u32().unwrap();
        assert_eq!(created, modified);
        assert_ne!(modified, 0);
        assert!(r.boolean().unwrap());
        assert_eq!(r.u64().unwrap(), 0);
        assert_eq!(r.string(128).unwrap(), "configs_s.info");
        assert_eq!(r.string(50).unwrap(), "");
        r.finish().unwrap();
        let mut get = Writer(vec![10, 3, 21]);
        get.string("");
        get.string("configs_s.info");
        get.0.push(0);
        let records = server.handle(&get.0, &s, 1).await.unwrap().result.unwrap();
        let mut raw = records[0].clone();
        raw.push(0);
        let mut r = task::Reader::new(&raw);
        assert_eq!(r.blob().unwrap(), encrypted);
        r.finish().unwrap();
        let mut unsupported = query;
        unsupported[14] = 1;
        assert_eq!(
            server.handle(&unsupported, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        for n in 0..query.len() {
            assert!(server.handle(&query[..n], &s, 1).await.is_err());
        }
    }

    #[tokio::test]
    async fn only_authenticated_profile_lookup_gets_target_missing_profile_error() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(u64::MAX);
        let mut public = Writer(vec![8, 3, 1]);
        public.u64(s.user_id);
        public.0.push(0);
        assert_eq!(
            server.handle(&public.0, &s, 1).await.unwrap().result,
            Err(task::NO_PROFILE)
        );
        assert_eq!(
            server.handle(&[8, 3, 2, 0], &s, 1).await.unwrap().result,
            Err(task::NO_PROFILE)
        );
        assert_eq!(
            server
                .handle(&public.0, &session(1), 1)
                .await
                .unwrap()
                .result,
            Err(task::TASK_FAILED)
        );
        assert!(server.handle(&[8, 3, 2, 0, 0], &s, 1).await.is_err());
    }

    #[tokio::test]
    async fn public_profile_persists_restarts_and_rejects_bad_crc_without_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let server = LocalServices::open(root.clone()).await.unwrap();
        let s = session(u64::MAX);
        let arguments = include_bytes!("../../../fixtures/protocol/public-profile-synthetic.bin");
        let mut upload = vec![8, 3, 3];
        upload.extend_from_slice(arguments);
        assert!(
            server
                .handle(&upload, &s, 1)
                .await
                .unwrap()
                .result
                .unwrap()
                .is_empty()
        );
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let mut get = Writer(vec![8, 3, 1]);
        get.u64(s.user_id);
        get.0.push(0);
        let expected = profile::public_record(s.user_id, arguments).unwrap();
        assert_eq!(
            server
                .handle(&get.0, &s, 2)
                .await
                .unwrap()
                .result
                .unwrap()
                .as_slice(),
            std::slice::from_ref(&expected)
        );
        upload[14] ^= 1;
        assert!(server.handle(&upload, &s, 3).await.is_err());
        assert_eq!(
            server.handle(&get.0, &s, 4).await.unwrap().result.unwrap(),
            [expected]
        );
        assert_eq!(
            server.handle(&get.0, &session(1), 4).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let mut other = Writer(vec![8, 3, 1]);
        other.u64(1);
        other.0.push(0);
        assert_eq!(
            server
                .handle(&other.0, &session(1), 4)
                .await
                .unwrap()
                .result,
            Err(task::NO_PROFILE)
        );
    }

    #[tokio::test]
    async fn empty_lab_catalogue_only_answers_supported_unfiltered_queries() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let mut query = Writer(vec![80, 3, 70]);
        query.string("");
        query.u32(1);
        query.u32(2000);
        query.boolean(true);
        query.u32(0);
        query.u32(0);
        query.string("982808");
        query.0.push(0);
        assert_eq!(
            server.handle(&query.0, &s, 1).await.unwrap().result,
            Ok(vec![])
        );
        let mut links = Writer(vec![80, 3, 107]);
        links.string("");
        links.u32(1);
        links.u32(2000);
        links.string("982808");
        links.0.push(0);
        assert_eq!(
            server.handle(&links.0, &s, 1).await.unwrap().result,
            Ok(vec![])
        );
        for n in 0..links.0.len() {
            assert!(server.handle(&links.0[..n], &s, 1).await.is_err());
        }
        for n in 0..query.0.len() {
            assert!(server.handle(&query.0[..n], &s, 1).await.is_err());
        }
        let mut page = query.0.clone();
        page[6] = 2;
        assert_eq!(
            server.handle(&page, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let mut filter = query.0;
        filter[18] = 1;
        assert_eq!(
            server.handle(&filter, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
    }
}
