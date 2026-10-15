# 本地管理面板

CODOL Control 是 Rust 可执行文件内置的本地 Web 管理界面。无需安装 Node 或前端依赖。

在 `server/` 下启动：

```sh
sh tools/cargo.sh run --locked -p codol-server -- admin
```

打开 `http://127.0.0.1:39080`。Ctrl-C 关闭面板。可通过 `--bind 127.0.0.1:39081` 更换本地端口，`--config` 指定观察服务的 TOML 文件。

读取现有 GameHub 后端日志：

```sh
sh tools/cargo.sh run --locked -p codol-server -- admin \
  --config configs/local.toml \
  --log-file "research-output/你的运行目录/tls-status.log"
```

`--log-file` 需替换为实际运行日志路径；也可以读取 `observer.log`。默认显示面板操作日志，日志文件不存在时显示读取提示。面板每 3 秒刷新，读取文件最新 128 KiB 内最多 500 行，面板操作记录保留最新 200 条。暂停更新仅冻结当前页面的日志视图，导出使用当前筛选结果。文件追加、截断或替换后，后续刷新读取新内容。

界面包含服务概览、交互终端、运行日志和服务配置。概览中的运行时间属于管理面板；监听端点来自配置，不主动连接研究端口，也不表示 AUTH、游戏或地图资源已就绪。服务实际状态请结合对应服务日志确认。

终端支持 `help`、`status`、`listeners`、`check-config`、`show-config`、`tail` 和 `clear`，提供命令历史及 Tab 补全。⌘/Ctrl-K 打开终端。此处是内置诊断控制台，不执行任意系统 shell，也不启动、重启或停止游戏进程。

配置页读写启动时指定的观察配置文件。校验沿用 `Config::validate`，保留 TOML 注释和原始文本。保存前检查 SHA-256 文件版本，原文件备份到同目录的 `*.toml.backup-*`，通过同目录临时文件替换配置，并保留原文件权限。外部修改导致版本冲突时保留草稿，可导出后重新加载。⌘/Ctrl-S 保存。配置保存后需通过原启动流程重新启动观察服务才能生效；AUTH、游戏服务的 CLI 参数和 publisher 文件不属于此编辑器。

面板只允许绑定 loopback，校验请求 Host 与 Origin，写入接口要求 JSON；未提供远程访问和账号认证。页面资源随二进制编译内置，修改 `crates/server/src/admin/` 后需重新编译。

验证：

```sh
sh tools/cargo.sh test --locked -p codol-server admin::tests
```
