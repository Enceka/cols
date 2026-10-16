//! Build a self-contained, validated server resource directory without account data.
mod bundle;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, ensure};
pub(crate) use bundle::{Progress, prepare};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    #[default]
    Catalog,
    Captures,
    Client,
}
fn default_mode() -> String {
    "war".into()
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub reuse_stun: bool,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub maps: Vec<String>,
    pub source: PathBuf,
    pub output_parent: PathBuf,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.reuse_stun || self.kind == Kind::Client,
            "共用发现服务只适用于本机采集"
        );
        ensure!(
            matches!(self.mode.as_str(), "war" | "cha") && self.maps.len() <= 64,
            "地图模式或数量无效"
        );
        for map in &self.maps {
            ensure!(
                crate::content::maps::registry::supports_mode(map, &self.mode),
                "地图不支持所选模式"
            );
        }
        if self.kind == Kind::Client {
            ensure!(
                cfg!(target_os = "macos") && self.mode == "war",
                "本机自动采集目前支持macOS/GameHub的战术模式；其它环境可导入本地采集结果"
            );
        }

        for path in [&self.source, &self.output_parent] {
            ensure!(
                path.is_absolute()
                    && path
                        .to_str()
                        .is_some_and(|s| !s.contains(['\0', '\r', '\n'])),
                "资源路径必须是有效的绝对路径"
            );
        }
        ensure!(self.source.exists(), "资源来源不存在");
        if self.kind == Kind::Client && self.output_parent.exists() {
            ensure!(
                !self
                    .output_parent
                    .canonicalize()?
                    .starts_with(self.source.canonicalize()?),
                "采集输出不能位于客户端目录内"
            );
        }
        if self.output_parent.exists() {
            ensure!(self.output_parent.is_dir(), "资源输出位置不是目录");
        }
        Ok(())
    }
}

pub(crate) fn source_catalog(source: &std::path::Path) -> Result<PathBuf> {
    if source.is_file() {
        return source.canonicalize().context("解析资源目录文件");
    }
    for name in [
        "catalog.json",
        "native-tdm-map-catalog.json",
        "native-three-map-catalog.json",
    ] {
        let path = source.join(name);
        if path.is_file() {
            return path.canonicalize().context("解析资源目录文件");
        }
    }
    anyhow::bail!("来源中没有地图目录；请选择已有地图目录JSON，或先从只读采集生成目录")
}

pub(crate) fn publish_progress(path: &std::path::Path, progress: &Progress) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("progress.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    serde_json::to_writer(&mut file, progress)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn helper() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let parent = executable.parent().context("服务程序没有父目录")?;
    for root in [
        std::env::current_dir()?,
        parent.to_path_buf(),
        parent.parent().unwrap_or(parent).to_path_buf(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
    ] {
        let script = root.join("tools/prepare_resources.py");
        if script.is_file() {
            return script.canonicalize().context("解析本地准备工具");
        }
    }
    anyhow::bail!(
        "本机采集／转换需要随程序提供的tools/prepare_resources.py；现有地图目录导入不需要Python"
    )
}
