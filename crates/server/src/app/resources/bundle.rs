use crate::game::session::{catalog::GameMapCatalog, prepared::PreparedGameState};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const LINKS: [&str; 7] = [
    "arsenal_file",
    "collision_file",
    "appearance_file",
    "voice_file",
    "props_file",
    "radar_file",
    "nuke_file",
];
const MAX_ASSET: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Clone, Serialize)]
pub(crate) struct Progress {
    pub completed: usize,
    pub total: usize,
    pub message: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    entries: Vec<Entry>,
    #[serde(default)]
    rotations: Vec<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    map: String,
    mode: String,
    file: PathBuf,
}
struct Staging {
    path: PathBuf,
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut file = new_file(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
fn read_json(path: &Path, limit: u64) -> Result<Value> {
    ensure!(
        path.is_file() && path.metadata()?.len() <= limit,
        "资源文件缺失或超出预算：{}",
        path.display()
    );
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "资源文件增长超出预算");
    serde_json::from_slice(&bytes).with_context(|| format!("资源JSON无效：{}", path.display()))
}
fn hash_file(path: &Path, cancelled: &impl Fn() -> bool) -> Result<(String, u64)> {
    ensure!(
        path.is_file() && path.metadata()?.len() <= MAX_ASSET,
        "引用资源缺失或超过256MiB：{}",
        path.display()
    );
    let mut source = File::open(path)?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut bytes = [0; 65536];
    loop {
        ensure!(!cancelled(), "资源准备已取消");
        let n = source.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(total <= MAX_ASSET, "资源增长超出预算");
        hash.update(&bytes[..n]);
    }
    Ok((format!("{:x}", hash.finalize()), total))
}

pub(crate) fn prepare(
    source: &Path,
    output: &Path,
    mut progress: impl FnMut(Progress) -> Result<()>,
    cancelled: impl Fn() -> bool,
) -> Result<Value> {
    ensure!(!output.exists(), "资源输出目录已存在，拒绝覆盖");
    let catalog_path = super::source_catalog(source)?;
    verify_manifest(&catalog_path, &cancelled)?;
    let raw = read_json(&catalog_path, 65536)?;
    let catalog: Manifest = serde_json::from_value(raw).context("地图目录格式无效")?;
    ensure!(
        catalog.schema_version == 1 && (1..=64).contains(&catalog.entries.len()),
        "地图目录为空或版本／数量无效"
    );
    let source_root = catalog_path.parent().context("地图目录没有父目录")?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let output = parent.join(output.file_name().context("资源目录名缺失")?);
    ensure!(
        output != source_root && !source_root.starts_with(&output),
        "输出目录不能覆盖来源"
    );
    let mut random = [0; 12];
    openssl::rand::rand_bytes(&mut random)?;
    let staging = Staging {
        path: parent.join(format!(
            ".resources-{}.pending",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )),
    };
    std::fs::create_dir(&staging.path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging.path, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::create_dir(staging.path.join("assets"))?;
    std::fs::create_dir(staging.path.join("maps"))?;
    let mut identities = BTreeSet::new();
    let mut assets = BTreeMap::new();
    let mut entries = Vec::new();
    let mut files = Vec::new();
    let mut total_bytes = 0;
    let count = catalog.entries.len();
    for (ordinal, entry) in catalog.entries.into_iter().enumerate() {
        ensure!(!cancelled(), "资源准备已取消");
        ensure!(
            crate::content::maps::registry::supports_mode(&entry.map, &entry.mode)
                && identities.insert((entry.map.clone(), entry.mode.clone())),
            "地图身份、模式或重复条目无效"
        );
        progress(Progress {
            completed: ordinal,
            total: count,
            message: format!("正在准备 {} / {}", entry.map, entry.mode),
        })?;
        let state_path = source_root
            .join(&entry.file)
            .canonicalize()
            .with_context(|| format!("找不到 {} 的准备文件", entry.map))?;
        let mut state = read_json(&state_path, 256 * 1024)?;
        ensure!(
            state["schema_version"] == 1
                && state["client_sha256"] == codol_protocol::CLIENT_SHA256
                && state["map"] == entry.map
                && state["mode"] == entry.mode,
            "地图准备身份不匹配"
        );
        let object = state.as_object_mut().context("地图准备必须为对象")?;
        for (key, value) in object.iter_mut() {
            if !key.ends_with("_file") || value.is_null() {
                continue;
            }
            ensure!(LINKS.contains(&key.as_str()), "未知资源引用字段：{key}");
            let reference = value.as_str().context("资源引用不是路径")?;
            let origin = state_path
                .parent()
                .unwrap()
                .join(reference)
                .canonicalize()
                .with_context(|| format!("{} 缺少 {key}", entry.map))?;
            ensure!(
                !origin.starts_with(&staging.path),
                "资源不能引用正在生成的目录"
            );
            let (hash, size) = hash_file(&origin, &cancelled)?;
            let relative = format!("assets/{hash}.json");
            if let std::collections::btree_map::Entry::Vacant(asset) = assets.entry(hash.clone()) {
                total_bytes += size;
                ensure!(total_bytes <= MAX_TOTAL, "资源包超过2GiB预算");
                let destination = staging.path.join(&relative);
                let mut source = File::open(&origin)?;
                let mut out = new_file(&destination)?;
                let mut copied_hash = Sha256::new();
                let mut copied = 0;
                let mut bytes = [0; 65536];
                loop {
                    ensure!(!cancelled(), "资源准备已取消");
                    let n = source.read(&mut bytes)?;
                    if n == 0 {
                        break;
                    }
                    copied += n as u64;
                    ensure!(copied <= MAX_ASSET, "资源增长超出预算");
                    out.write_all(&bytes[..n])?;
                    copied_hash.update(&bytes[..n]);
                }
                out.sync_all()?;
                ensure!(
                    copied == size && format!("{:x}", copied_hash.finalize()) == hash,
                    "来源在复制期间发生变化"
                );
                files.push(json!({"file":relative,"sha256":hash,"bytes":size}));
                asset.insert(relative.clone());
            }
            *value = json!(format!("../{relative}"));
        }
        let name = format!("maps/{}-{}.json", entry.map, entry.mode);
        write_json(&staging.path.join(&name), &state)?;
        let (hash, bytes) = hash_file(&staging.path.join(&name), &cancelled)?;
        files.push(json!({"file":name,"sha256":hash,"bytes":bytes}));
        // The normal runtime loader validates CRC, map identity and all referenced resources.
        PreparedGameState::load_catalog_entry(&staging.path.join(&name))
            .with_context(|| format!("{} / {} 资源校验失败", entry.map, entry.mode))?;
        entries.push(json!({"map":entry.map,"mode":entry.mode,"file":name}));
    }
    progress(Progress {
        completed: count,
        total: count,
        message: "正在校验完整地图目录".into(),
    })?;
    write_json(
        &staging.path.join("catalog.json"),
        &json!({"schema_version":1,"entries":entries,"rotations":catalog.rotations}),
    )?;
    GameMapCatalog::load(&staging.path.join("catalog.json"))?;
    let (hash, bytes) = hash_file(&staging.path.join("catalog.json"), &cancelled)?;
    files.push(json!({"file":"catalog.json","sha256":hash,"bytes":bytes}));
    ensure!(!cancelled(), "资源准备已取消");
    let default_file = entries
        .iter()
        .find(|e| e["mode"] == "war")
        .unwrap_or(&entries[0])["file"]
        .as_str()
        .context("默认地图缺失")?;
    let manifest = json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"catalog":"catalog.json","game_state":default_file,"maps":entries.len(),"asset_files":assets.len(),"asset_bytes":total_bytes,"files":files,"runtime_validated":true,"native_acceptance_claimed":false});
    write_json(&staging.path.join("resources.json"), &manifest)?;
    ensure!(!output.exists(), "资源输出目录已存在，拒绝覆盖");
    std::fs::rename(&staging.path, &output)?;
    Ok(
        json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"pid":std::process::id(),"map_catalog":output.join("catalog.json"),"game_state":output.join(default_file),"output":output,"maps":entries.len(),"asset_files":assets.len(),"asset_bytes":total_bytes,"runtime_validated":true}),
    )
}

fn verify_manifest(catalog: &Path, cancelled: &impl Fn() -> bool) -> Result<()> {
    let root = catalog.parent().context("资源目录没有父目录")?;
    let path = root.join("resources.json");
    if !path.exists() {
        return Ok(());
    }
    let manifest = read_json(&path, 1024 * 1024)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["client_sha256"] == codol_protocol::CLIENT_SHA256,
        "资源清单身份不匹配"
    );
    let files = manifest["files"]
        .as_array()
        .context("资源清单缺少校验列表")?;
    ensure!((1..=1024).contains(&files.len()), "资源清单数量无效");
    let mut seen = BTreeSet::new();
    for file in files {
        let relative = Path::new(file["file"].as_str().context("资源清单路径无效")?);
        ensure!(
            !relative.is_absolute()
                && relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && seen.insert(relative.to_path_buf()),
            "资源清单路径重复或越界"
        );
        let actual = root.join(relative).canonicalize()?;
        ensure!(actual.starts_with(root), "资源清单不能引用目录之外的文件");
        let (hash, bytes) = hash_file(&actual, cancelled)?;
        ensure!(
            file["sha256"] == hash && file["bytes"] == bytes,
            "资源校验和不匹配：{}",
            relative.display()
        );
    }
    ensure!(
        seen.contains(Path::new("catalog.json")) && catalog == root.join("catalog.json"),
        "资源清单未覆盖地图目录"
    );
    Ok(())
}
