# 工程基础与运行手册

更新：2026-10-06。本文件保留工程基础说明；认证与传输见 [AUTH_LOBBY_PROGRESS.md](AUTH_LOBBY_PROGRESS.md)，最新文件服务与公共配置阻塞点见 [SERVICE_INITIALIZATION.md](SERVICE_INITIALIZATION.md)。玩家档案、背包和完整大厅界面仍未完成。

后续已完成公共档案真实重登、空目录与余额/附件初始化；当前接续请读 [ONBOARDING_PROGRESS.md](ONBOARDING_PROGRESS.md)。41 项 Rust、5 项 Python 测试通过；下文描述基础观测/存储阶段。

**现有实现**

| 路径 | 内容 |
| --- | --- |
| `crates/protocol` | 配置、TLS/AUTH、票据、协议 210 握手与加密帧、UDP 发现、tagged 任务及回执 |
| `crates/server` | 观察器、TLS/HTTP AUTH、票据绑定、大厅与 STUN、本地时间/文件服务、捕获与协作退出 |
| `configs/local.toml` | 两个仅监听回环地址的研究端点；端口不代表官方值 |
| `fixtures/client-3.3.3.3.json` | 9 个文件的固定指纹、7 个字符串锚点 |
| `fixtures/resources-3.3.3.3.json` | 362 个本地 IIPS 文件的大小和 SHA-256，属于本地参考值 |
| `tools/verify_client.py` | 样本哈希、版本清单、资源文件名/大小/文件头检查，可选全量资源哈希比较 |
| `tools/analyze.py` | 启动器 CIL 解码、指定 RVA 的 x86 反汇编、可执行节中的绝对地址候选引用 |
| `tools/check_capture.py` | 采集事件连续性、结束记录、字节数和内容哈希核验 |
| `tools/check.sh` | 格式、Clippy、Rust 和 Python 测试的统一入口 |
| `.github/workflows/check.yml` | macOS/Linux CI 定义；本轮只执行了本机验证，尚未运行远程 CI |

observe 命令记录原始 TCP 字节流。操作系统的一次 `read` 可能对应半个、一个或多个协议消息，`stream_chunk` 不代表游戏报文边界；该命令仍不回包。serve-auth 则已支持 Rust 进程内的 OpenSSL TLS/HTTP、100 Continue、本地票据、协议 210 及可选 STUN/业务服务。

--service-data-dir 启用时间、透明用户文件持久化和操作者提供的公共文件。未知任务明确失败，以保持客户端 FIFO；默认观察模式只回复首条注册任务。玩家/背包模型、SQLite 及完整大厅业务尚未实现。真实客户端当前因必需公共配置缺失退出。

**构建与检查**

固定 Rust 1.96.0，依赖解析记录在 Cargo.lock。此机器的 Homebrew `rustc 1.97.1` 因 LLVM 符号不匹配启动失败；已使用 rustup 安装独立的 1.96.0，并通过项目脚本显式设置 PATH、RUSTC 和 RUSTDOC。没有替换 Homebrew 安装或更改全局 Rust 默认值。

```sh
sh tools/check.sh
sh tools/cargo.sh build --locked --workspace
sh tools/cargo.sh run --locked -p codol-server -- check-config
```

当前通过 32 项 Rust、5 项 Python 测试，覆盖观察器、AUTH/TLS、票据独立向量、大厅认证与加密、UDP、任务 FIFO、文件持久化与身份隔离。socket 测试使用合成流量；真实客户端实验单独记录，不能由这些测试替代。

另已对编译后的 CLI 做独立进程验证：启动两个临时 TCP 端口、发送合成二进制、确认没有应用层回包、发送 SIGINT、检查进程正常退出并核对完整采集；生成配置及拒绝覆盖已有文件也已验证。本地结果摘要位于 research-output/cli-smoke-result.json，临时监听器均已关闭。

**样本及研究工具**

```sh
python3 tools/verify_client.py > research-output/client-verification.json
python3 tools/verify_client.py --hash-resources > research-output/client-verification-full.json
.venv/bin/python tools/analyze.py launcher > research-output/launcher-il.json
.venv/bin/python tools/analyze.py native --rva 0x241efc --size 0x8c > research-output/dwauth-reader.json
.venv/bin/python tools/analyze.py refs --va 0x3746910 > research-output/endpoint-b-refs.json
```

全量资源比较要读取约 10 GB 文件，得到的是“与本地参考样本一致”，不是官方完整性验证。当前尚未验证 IIPS 内部索引、缓存块或资源解压结果。安装器本身已计算哈希，但没有运行或解包验证。

原生工具需要调用者提供正确的指令起始 RVA；输出中的 `instruction_boundary_verified=false` 提醒该工具不会自动证明函数边界。`refs` 是绝对 32 位字节模式搜索，可能误命中指令或数据，必须继续检查周围代码。每次分析前工具均核对目标样本哈希，阻止把旧地址用于其他版本。

使用的第三方 Python 工具为 [dnfile](https://github.com/malwarefrank/dnfile)、[dncil](https://github.com/mandiant/dncil)、pefile 与 Capstone，版本固定于 tools/requirements.txt。完整 CIL/反汇编输出保存在本地 research-output，主要结论及重现命令保存在 docs。

**运行观测服务**

```sh
sh tools/cargo.sh run --locked -p codol-server -- observe --config configs/local.toml
```

启动时检查全部监听地址；任一绑定失败就整体失败，不会只启动一半端点。配置中的 capture_dir 相对于 TOML 文件所在目录解释。监听地址必须是明确的 IP 和端口。研究用客户端副本可以连接这些地址，但收到的只有正常 TCP 行为，没有游戏认证响应。

默认限制：全局同时 16 个会话，一次运行最多记录 128 个会话，每会话最多 1 MiB、4096 次读取，空闲 10 秒、总时长 30 秒。达到单会话限制会关闭该连接并记录原因；达到运行会话总数后停止接受新采集会话，使用 Ctrl-C 结束这次运行。并发超过限制时立即丢弃新接受的连接。每次运行创建新目录，已有数据不被覆盖。

输出格式：

```text
captures/run-<unix_ms>-<pid>/
  run.json                     版本、目标样本、配置与实际监听地址
  000001-endpoint_a.bin         此连接的 client_to_server 连续字节
  000001-endpoint_a.jsonl       开始、读取区间、结束及内容 SHA-256
```

`target_client_sha256` 仅标识实验预期样本，不证明连接对端的实际程序身份。对端地址、连接号及端点名用于区分会话。`peer_eof`、`shutdown`、`byte_limit`、`chunk_limit`、`idle_timeout`、`session_timeout`、`read_error` 是可能的结束原因。限额或退出造成的截断不能当成完整游戏消息。

Ctrl-C 会让已有记录完成文件刷新和结束事件。磁盘错误会使运行失败；缺少结束事件的记录是未完成采集。这里的 flush 不提供断电时的磁盘持久性保证。校验一次已停止的运行：

```sh
python3 tools/check_capture.py captures/run-实际时间-实际进程号
```

采集核验只证明保存内容的连续性及哈希一致，不证明协议正确。进行中的记录通常尚无结束事件，应在停止运行后校验。

**生成实验配置**

```sh
mkdir -p research-output
sh tools/cargo.sh run --locked -p codol-server -- bootstrap \
  --config configs/local.toml \
  --host 127.0.0.1 \
  --title-id-hex 0 \
  --output research-output/dwauth.ini
```

这里的 `0` 是研究取值，不能据此期待认证成功。字段现已动态确认为 title_id，端点 B 已确认 HTTPS AUTH，端点 A 职责尚未确认。工具强制显式提供 title ID 并拒绝覆盖已有文件；旧选项 --context-hex 保留为别名，不会自动修改 CODOL 原件。

只有在运行副本和端点配置就绪后，才将实验文件放入该副本。虚拟机中的 `127.0.0.1` 指向虚拟机自身；如果客户端在虚拟机而服务在 Mac 上，需配置实际可达的宿主机地址，并相应调整监听地址。

**环境与下一步**

已确认：Mac 为 arm64，Rust 原生构建及 Java 17/21/25 可用，静态工具在 .venv。用户随后准备好了 Parallels，Windows 状态已变为 running，当前用户来宾命令可用。本轮主要改用已有 GameHub 引擎 10，已运行客户端研究副本到 HTTPS AUTH；原启动器和 CrossProxy 没有在本轮执行。

下一个实质目标：恢复目标版必需公共配置 configs_s.info 和 release playlist 的解密及内容解析，继续推进目录、玩家档案与背包；具体入口见 SERVICE_INITIALIZATION.md。
