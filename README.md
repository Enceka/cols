# CODOL 3.3.3.3 / Rust

面向固定 CODOL 3.3.3.3 客户端的纯 Rust 本地服务端，包含认证、大厅、档案与库存、房间、原生游戏通道及权威对局。客户端沿用本机合法持有的 Windows x86 程序，通过 GameHub Wine/Proton 运行；游戏资源、账号数据和密钥不随仓库分发。

已具备战术／冲锋团队竞技、Bot 战斗与补位、原生选图、移动与飞扑、射击与投掷物、死亡重生、回放和结算。本机双客户端及远端 Linux 成品联调已验证同房、真人移动／交火、一致结算和再次开局。地图资源、单人 Bot 对局、共享多人对局各有独立验收范围，详见[功能状态](docs/REFERENCE_ALIGNMENT.md)。

连杀进度、循环发奖、库存和部分奖励效果已接入；经验结算需要显式策略，剧情仅支持资源初始化与采集。未完成的远控载具、完整剧情、装备及表现范围见[待完成与体验复核](docs/REVIEW_PENDING.md)。

## 构建与检查

在本仓库根目录执行：

```sh
rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy
sh tools/check.sh
sh tools/cargo.sh build --locked --workspace --release
sh tools/cargo.sh run --locked -p codol-server -- check-config
```

`tools/cargo.sh` 选择项目固定工具链。普通测试使用合成数据；依赖本机游戏捕获的检查须按专题显式运行。构建方法、Python 环境和诊断命令见[工程运行手册](docs/FOUNDATION.md)。

## 运行

初次使用请读[使用教程](docs/USER_GUIDE.md)，按步骤配置启动器、选择玩家并进入对局。已有本机客户端、服务资料及地图时，也可按[本地游玩](docs/LOCAL_PLAY.md)快捷启动。正常准备使用 7500 分／10 分钟，运行会话持续至退出。资源和本机路径需要自行准备，仓库不能直接充当便携游戏包。

仅观察 TCP 字节流：

```sh
sh tools/cargo.sh run --locked -p codol-server -- observe
```

默认观察端口为 `127.0.0.1:39001`、`127.0.0.1:39002`，不会提供游戏响应。实际游戏服务使用 `serve-auth`，参数与启动契约见[原生启动器接口](docs/NATIVE_LAUNCHER.md)。

本地管理面板：

```sh
sh tools/cargo.sh run --locked -p codol-server -- admin
```

访问 `http://127.0.0.1:39080`；功能及配置生效范围见[管理面板](docs/ADMIN_PANEL.md)。

## 文档

[文档索引](docs/README.md)覆盖全部专题；开发约束见 [AGENT.md](AGENT.md)，模块归属见[服务端结构](docs/SERVER_MODULARIZATION_PLAN.md)，样本和证据方法见[客户端基线](docs/CLIENT_BASELINE.md)。使用与分发边界见 [LEGAL.md](LEGAL.md)。
