# CODOL 3.3.3.3 / Rust

当前纯 Rust 后端已通过真实 3.3.3.3 客户端验证：认证、大厅、训练场景、玩家背包装配、切枪、换弹、瞄准和胶囊碰撞移动。基础声音已恢复；训练靶支持遮挡检查、扣血、销毁和重生，世界弹着事件已接通。AI、多人世界同步和完整对局规则仍待实现；商城价格与购买按用户要求暂缓。

从 [docs/README.md](docs/README.md) 接续任务；运行细节见 [docs/FOUNDATION.md](docs/FOUNDATION.md)，静态发现见 [docs/BOOTSTRAP_FINDINGS.md](docs/BOOTSTRAP_FINDINGS.md)。

最新动态结果、GameHub 运行命令及 AUTH 响应逆向入口见 [docs/GAMEHUB_DEBUG.md](docs/GAMEHUB_DEBUG.md)。

最新完成情况与可重现命令见 [docs/AUTH_LOBBY_PROGRESS.md](docs/AUTH_LOBBY_PROGRESS.md)。

最新业务层进展、文件持久化测试及公共配置阻塞点见 [docs/SERVICE_INITIALIZATION.md](docs/SERVICE_INITIALIZATION.md)。

当前接续入口为 [docs/COMBAT_PROGRESS.md](docs/COMBAT_PROGRESS.md)：记录本版命中、弹着材质、实体坐标修复与实机验收。此前出生和装备证据见 [docs/PLAYER_BOOTSTRAP_PROGRESS.md](docs/PLAYER_BOOTSTRAP_PROGRESS.md)。

在 `server/` 下执行：

```sh
rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy
sh tools/check.sh
sh tools/cargo.sh run --locked -p codol-server -- check-config
sh tools/cargo.sh run --locked -p codol-server -- observe
```

默认监听 `127.0.0.1:39001`、`127.0.0.1:39002`；它们是研究端口。Ctrl-C 关闭连接、刷新采集文件并退出。`tools/cargo.sh` 显式选择 rustup 编译器，以处理本机 Homebrew Rust/LLVM 不匹配的问题，不修改系统默认工具链。

Rust 测试使用合成字节及本机 TCP 连接，不需要游戏资源。静态研究工具使用单独的 Python 环境：

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r tools/requirements.txt
mkdir -p research-output
python3 tools/verify_client.py > research-output/client-verification.json
.venv/bin/python tools/analyze.py launcher > research-output/launcher-il.json
```

默认从仓库上一级读取用户提供的客户端，工具支持 `--workspace` 指定其他目录。原始游戏程序和资源不复制进此仓库；完整分析输出、运行副本和网络采集默认不纳入版本控制。
