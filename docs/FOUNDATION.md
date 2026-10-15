# 工程基础与运行手册

目标为固定 3.3.3.3 客户端与纯 Rust 服务端。运行能力和验收范围见[功能状态](REFERENCE_ALIGNMENT.md)，完整文档入口见[索引](README.md)。所有命令从 `server/` 根目录运行。

## 工程与依赖

| 路径 | 职责 |
| --- | --- |
| `crates/protocol/` | AUTH、协议 210、任务编码、游戏加密通道、gamestate、快照和输入编解码 |
| `crates/server/src/` | 应用组合、业务、资源、玩法、传输、存储和管理面板 |
| `configs/` | 观察服务 TOML 与公共配置模板 |
| `fixtures/` | 固定客户端指纹、脱敏协议向量与必要资源元数据 |
| `tools/` | 构建检查、资源准备、只读探针及原生客户端实验工具 |
| `client-work/`、`research-output/` | 忽略的客户端副本、私有服务资料、采集及验证结果 |

Rust 固定为 1.96.0，依赖由 `Cargo.lock` 锁定。`tools/cargo.sh` 显式选择 rustup 的编译器，避免本机其它 Rust/LLVM 安装影响构建。需要 OpenSSL；静态分析工具依赖单独的 Python 环境。源码职责和依赖方向见[模块结构](SERVER_MODULARIZATION_PLAN.md)。

```sh
rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy
python3 -m venv .venv
.venv/bin/python -m pip install -r tools/requirements.txt
mkdir -p research-output
sh tools/cargo.sh build --locked --workspace --release
sh tools/check.sh
```

`check.sh` 运行架构检查、fmt、严格 Clippy、Rust 测试与 Python 测试。普通测试不要求游戏资源；`ignored` 资源检查按专题提供本机资料后显式执行。测试通过只证明对应行为，不替代原生画面、听感或有效业务决策。

所有构建在本机完成；Linux 使用交叉编译，云主机只部署和联调。远端方法及边界见[Linux 验收](REMOTE_LINUX_ACCEPTANCE.md)。

## 游戏运行与数据

[本地游玩](LOCAL_PLAY.md)提供已有环境的入口；[本地新玩家](LOCAL_PLAYER.md)说明独立账号模板和存档。[原生启动器接口](NATIVE_LAUNCHER.md)定义服务就绪契约。`serve-auth --help` 列出认证、大厅、STUN、游戏 UDP、地图目录和 Bot 配置；缺少本机资源时先按[地图准备](MAP_RESOURCE_PREPARATION.md)生成，不能借用其它地图 CRC。

文档中的 `EXPERIMENT_JSON`、`ASSETS_JSON`、`CAPTURE_JSON`、`GAME_STATE_JSON`、`CATALOG_JSON` 等是实际输入文件的占位符；`GAME_PREFIX`、`PROBE_PREFIX` 分别替换为已准备的游戏环境和探针构建环境。探针编译在其目标 prefix 没有客户端运行时执行，采样则要求实验仍在运行。只读探针不调用游戏函数、不改写游戏内存，见[客户端诊断](ONBOARDING_PROGRESS.md)。

## 观察与捕获

```sh
sh tools/cargo.sh run --locked -p codol-server -- check-config
sh tools/cargo.sh run --locked -p codol-server -- observe --config configs/local.toml
```

观察器记录 TCP 字节流，不回游戏协议包。一次 `read` 可以包含半个或多个消息，`stream_chunk` 不是报文边界。配置相对路径以 TOML 所在目录解析；监听器必须全部绑定成功。Ctrl-C 刷新捕获并关闭会话。

默认同时 16 个会话，单次最多 128 个会话，每会话最多 1 MiB／4096 次读取，空闲 10 秒、总时长 30 秒。这是观察器预算；正常游玩使用独立的持续运行模式。

```text
captures/run-<id>/
  run.json
  000001-endpoint_a.bin
  000001-endpoint_a.jsonl
```

```sh
python3 tools/check_capture.py CAPTURE_RUN_DIRECTORY
```

停止采集后核对连续性、结束事件、字节数和哈希；这些不能独自证明协议正确。`target_client_sha256` 标识预期样本，不证明对端程序身份。原始流量、票据、私钥及账号数据只保存在忽略目录。

## 样本研究

```sh
python3 tools/verify_client.py > research-output/client-verification.json
python3 tools/verify_client.py --hash-resources > research-output/client-verification-full.json
.venv/bin/python tools/analyze.py launcher > research-output/launcher-il.json
.venv/bin/python tools/analyze.py native --rva 0x241efc --size 0x8c
.venv/bin/python tools/analyze.py refs --va 0x3746910
```

全量资源哈希需要读取约 10 GB，只证明与本地参考一致。IIPS 内部块、最终覆盖视图和依赖闭包须独立验证。反汇编需要正确指令边界；绝对地址字节匹配只是候选，必须核对指令和消费者。

`bootstrap` 生成新的实验 `dwauth.ini`，不覆盖已有文件：

```sh
sh tools/cargo.sh run --locked -p codol-server -- bootstrap \
  --config configs/local.toml --host 127.0.0.1 \
  --title-id-hex 3ea --output research-output/dwauth.ini
```

本版 title ID 为 1002（`3ea`）。虚拟机内的 loopback 指向来宾自身，连接宿主机需提供实际可达地址。日常游戏优先使用启动器的配置备份、证书恢复和进程归属管理，不能直接覆盖原始客户端。
