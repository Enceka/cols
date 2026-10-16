use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::Ipv4Addr,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub service_data_dir: PathBuf,
    pub game_state: PathBuf,
    pub map_catalog: PathBuf,
    pub auth_port: u16,
    pub lobby_port: u16,
    pub stun_port: u16,
    pub stun_alt_port: u16,
    pub game_port: u16,
    pub resource_port: u16,
    pub room_bot_fill: u8,
}

impl Settings {
    pub fn defaults(workspace: &Path, state: &Path) -> Self {
        let existing = workspace.join("client-work/server-data");
        Self {
            service_data_dir: if existing.is_dir() {
                existing
            } else {
                state.join("service-data")
            },
            game_state: workspace.join("research-output/tdm-afghan-hud-state.json"),
            map_catalog: workspace.join("research-output/native-tdm-map-catalog.json"),
            auth_port: 39002,
            lobby_port: 39001,
            stun_port: 3074,
            stun_alt_port: 3078,
            game_port: 39004,
            resource_port: 39003,
            room_bot_fill: 12,
        }
    }

    pub fn validate(&self, admin_port: u16) -> Result<()> {
        ensure!(
            self.room_bot_fill == 0 || (4..=12).contains(&self.room_bot_fill),
            "补位人数必须为0（关闭）或4～12"
        );
        for path in [&self.service_data_dir, &self.game_state, &self.map_catalog] {
            ensure!(
                path.is_absolute()
                    && path
                        .to_str()
                        .is_some_and(|s| !s.contains(['\n', '\r', '\0'])),
                "请使用有效的绝对文件路径"
            );
        }
        for ports in [
            [self.auth_port, self.lobby_port, self.resource_port],
            [self.stun_port, self.stun_alt_port, self.game_port],
        ] {
            let mut seen = HashSet::new();
            for port in ports {
                ensure!(port == 0 || seen.insert(port), "同一协议的服务端口不能重复");
            }
        }
        ensure!(
            ![self.auth_port, self.lobby_port, self.resource_port].contains(&admin_port),
            "服务端口不能占用管理面板端口"
        );
        Ok(())
    }

    pub fn check_inputs(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (name, path) in [
            ("默认地图", &self.game_state),
            ("地图目录", &self.map_catalog),
        ] {
            if !path.is_file() {
                errors.push(format!("{name}文件不存在：{}", path.display()));
            }
        }
        if self.service_data_dir.exists() && !self.service_data_dir.is_dir() {
            errors.push("服务资料路径不是目录".into());
        }
        errors
    }

    pub fn address(&self, port: u16) -> String {
        format!("{}:{port}", Ipv4Addr::LOCALHOST)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    pub schema_version: u8,
    pub settings: Settings,
}

pub(super) fn load(path: &Path, fallback: Settings, port: u16) -> Result<Settings> {
    let settings = if path.exists() {
        ensure!(
            path.metadata()?.len() <= 65536 && !path.symlink_metadata()?.is_symlink(),
            "开服配置不是有效常规文件"
        );
        let saved: Saved = serde_json::from_slice(&std::fs::read(path)?).context("读取开服配置")?;
        ensure!(saved.schema_version == 1, "不支持的开服配置版本");
        saved.settings
    } else {
        fallback
    };
    settings.validate(port)?;
    Ok(settings)
}
