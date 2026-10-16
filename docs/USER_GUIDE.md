# 使用教程

本教程面向在本机启动 CODOL 3.3.3.3、配置玩家并进入 Bot 对局的使用者。已准备好这台电脑的运行环境时，可以直接从“启动游戏”开始。

## 运行前需要什么

当前已验证的本机环境为 macOS Apple Silicon、GameHub Wine10 和 Rust 服务端。源码仓库不包含游戏客户端和地图资源，首次下载代码后还需要准备以下资料：

| 内容 | 本机约定位置或要求 |
| --- | --- |
| 工作区 | 同级放置 `server/`、`launcher/` 和 `CODOL/`；`CODOL/` 可以指向本机客户端副本 |
| 客户端 | 固定版本 3.3.3.3，主程序为 `codoMP_client_shipRetail.exe` |
| 兼容环境 | 已安装的 GameHub Wine10，以及项目已有的独立 Wine prefix |
| 服务端 | `server/target/release/codol-server` |
| 服务资料 | `server/client-work/server-data/`，包含已有账号及公共配置 |
| 地图准备 | `server/research-output/native-tdm-map-catalog.json` 和 `tdm-afghan-hud-state.json`，以及其中引用的本机资源文件 |
| 本地证书 | `server/.tools/tls-lab/server.pem` 和 `key.pem`，须有效且互相匹配 |
| 新玩家模板 | 新建玩家时需要 `server/client-work/server-data/new-player-template.json` |

本机配置生成器会查找已有环境，不下载客户端、不安装 Wine，也不生成缺失的地图资源。换电脑时，需根据[地图资源准备](MAP_RESOURCE_PREPARATION.md)、[新玩家模板](LOCAL_PLAYER.md)和[运行手册](FOUNDATION.md)重新准备并校验资料。

从源码构建时需要 Rust 1.96.0。以下命令从工作区根目录开始执行：

```sh
rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy
cd server
sh tools/cargo.sh build --release --locked -p codol-server
cd ../launcher
sh tools/cargo.sh build --release --locked
```

构建完成后，运行程序不需要安装 Rust。图形启动器也不依赖 Python；使用本教程中的 `play.py` 备用入口时需要 Python 3。

## 启动游戏

以下启动器命令均在工作区的 `launcher/` 目录执行。

首次使用，生成本机配置：

```sh
target/release/cols-launcher native-init --workspace ..
```

生成的配置为 `profiles/native.local.json`。已有此文件时直接使用，不重复生成；命令会拒绝覆盖。配置内路径相对 JSON 所在目录解析，移动工作区后需检查客户端、Wine、服务资料和地图路径。

打开图形启动器：

```sh
target/release/cols-launcher gui --config profiles/native.local.json
```

1. 选择“本机原生游戏”。
2. 确认配置 JSON 和客户端路径正确。已有玩家可从玩家列表中选择。
3. 点击“检查 / 预览连接”，按提示处理缺失文件、路径错误或已有游戏占用。
4. 检查允许启动后，点击“启动本机游戏”。
5. 等待服务端完成地图加载，再等待游戏进入大厅。启动期间保留启动器窗口。

检查只确认启动条件，实际登录和进图需要游戏运行后确认。首次为某张地图创建对局时，还可能生成并缓存导航数据。

也可直接使用命令行：

```sh
target/release/cols-launcher native-plan --config profiles/native.local.json
target/release/cols-launcher native-launch --config profiles/native.local.json
```

默认运行到退出。只想做有界启动检查时，可以添加 `--seconds 300`；该期限包含服务加载和客户端启动时间。

### 已有环境的快捷入口

在 `server/` 目录运行：

```sh
python3 tools/play.py
```

脚本先构建服务端，再使用已有本机账号和单真人 Bot 配置启动游戏，持续到退出。它不会选择启动器玩家列表中的独立玩家。需要指定另一套已准备环境时：

```sh
python3 tools/play.py \
  --prefix-name GAME_PREFIX \
  --map-catalog /path/to/map-catalog.json \
  --game-state /path/to/game-state.json
```

将 `GAME_PREFIX` 和两个路径替换为本机真实值。启动器与脚本每次选一个使用；同一客户端目录只允许一个受管会话。

## 创建或切换玩家

沿用已有账号时直接启动。需要独立存档时，在图形启动器的“新玩家配置”中：

1. 填写昵称，设置初始等级 1～70。
2. 按需勾选“全物品解锁”。
3. 点击“新建玩家并切换”，等待创建完成。
4. 确认当前玩家已切换，再检查并启动游戏。

昵称可以重名，实际登录身份和存档目录独立。等级与物品选项只在创建时生效；修改表单并再次创建会得到另一位玩家，不会修改已存在的存档。新玩家钱包为零，“全物品解锁”只授予物品所有权，武器等级、配件门槛和剧情解锁分别处理。

命令行创建示例：

```sh
target/release/cols-launcher native-create-player \
  --config profiles/native.local.json \
  --nickname "玩家一" --level 20 --unlock-all
```

命令会输出新玩家的配置路径。后续启动和异常恢复都应使用该路径：

```sh
target/release/cols-launcher native-launch --config /path/to/player/native.json
```

创建时提示缺少模板，先按[本地新玩家](LOCAL_PLAYER.md)准备模板。独立玩家创建与身份隔离已有自动验证，新玩家完整原生登录仍需专项验收；首次体验可优先沿用已验证的现有账号。

## 创建房间并开始一局

1. 进入游戏大厅，选择“玩 → 多人竞技 → 创建房间”。
2. 先选择“战术团队竞技”，再在房间内用地图预览两侧箭头选图。
3. 初次体验可选阿富汗或杀戮小屋（部分资料称安全屋），等待 Bot 加入和名单稳定。
4. 检查队伍和背包，点击“准备”。取消准备后可继续调整，切图后需要重新准备。
5. 等待加载进入地图；准备倒计时结束后开始战斗。

单真人、目标人数为 12 的房间会补齐 11 个 Bot，昵称为 `Bot 1`～`Bot 11`。图形启动器的 `backend.room_bot_fill` 默认 12，也可在配置 JSON 中设为 4～12；此项表示包含真人和 Bot 的总人数。`play.py` 使用固定单真人加 11 Bot 的兼容入口。

正常 12 人准备为双方各 6 人、7500 分或 10 分钟结束。入场有 5 秒冻结准备，可选背包；正式开战后有 15 秒未交战换包宽限。参与战斗或宽限结束后，换包通常在下次出生生效。

战术团队竞技使用随局势变化的复活点。冲锋团队竞技使用本队固定基地，同一地图的两种模式分别准备。当前图形启动器的自动补位只支持战术；体验已准备的冲锋 Bot 对局时，先退出启动器会话，再用 `play.py` 启动，在创建房间时选择“冲锋团队竞技”。可用地图见[地图覆盖](MAP_COVERAGE.md)和[冲锋团队竞技](CHARGE_MODE.md)。黑房通过独立“黑房人机”快速赛单进入；竞技场保留 4 人 TDM 诊断，完整 PVE 玩法暂缓。

一局结束后，按界面完成最终回放、前三名和比分结算，返回房间。需要再玩时重新准备，可以在同一会话中继续选图开局。

## 对局中的常用功能

按键以游戏“设置”里的当前绑定和屏幕提示为准，已有客户端设置会保留。

| 操作 | 使用方法 |
| --- | --- |
| 移动、射击、瞄准、切枪和换弹 | 使用当前游戏键位；可在设置中查看或调整 |
| 换背包 | 使用原生背包菜单；准备期、未交战宽限和下次出生有不同生效时机 |
| 拾取地面枪 | 走近并对准地面武器，按屏幕提示交互，默认使用 F |
| 跳过个人回放 | 按回放底部提示，默认 F；最终回放不提供同样的跳过提示 |
| 连杀奖励 | 获得库存后，按右侧图标显示的当前绑定调用；不同奖励和地图有各自开放范围 |
| 记分板 | 使用当前记分板按键，常见绑定为 Tab；Bot 延迟显示为 0 |

地图资源与具体效果未准备完整时，拥有物品或看到预览不代表该奖励可调用，详细范围见[连杀覆盖](KILLSTREAK_COVERAGE.md)。人物和武器经验需要服务端显式发奖策略，默认不能假定每局都会升级；剧情任务和文本聊天的完整玩法仍未开放。

## 正常退出与异常恢复

日常退出优先使用游戏菜单。也可以点击启动器“结束本次会话”；命令行运行时按 Ctrl-C。等待启动器完成清理，再关闭终端或重新启动。

启动器会停止本次服务并恢复临时连接配置与本地 CA，保留玩家存档和游戏设置。异常中断后，先退出仍运行的对应游戏，再使用“恢复上次配置”，或在 `launcher/` 下执行：

```sh
target/release/cols-launcher native-recover --config profiles/native.local.json
```

若使用独立玩家，`--config` 应换成该玩家启动时使用的 JSON。不要删除恢复记录和备份；恢复报告外部文件冲突时，先保留冲突文件并核对，详见[客户端启动与恢复](CLIENT_STARTUP_RECOVERY.md)。

## 常见问题

| 现象 | 处理方法 |
| --- | --- |
| 配置已存在 | 跳过 `native-init`，直接加载该 JSON；移动目录后检查其中路径 |
| 找不到客户端、Wine、证书或服务资料 | 按检查结果修正配置；生成配置不会补齐这些文件 |
| `prepared map catalog/state missing` | 检查地图目录、默认地图和引用文件；脚本可显式指定 `--map-catalog` 与 `--game-state` |
| 提示游戏或目录被占用 | 回到已运行会话，先正常退出并等待清理；换 Wine prefix 不能绕过同目录锁 |
| 服务端就绪超时 | 查看本次服务日志，确认地图文件有效、磁盘可读且端口未被其它会话占用；升级后重建服务端和启动器 |
| 没有窗口、加载卡住或探针超时 | 先按[启动恢复](CLIENT_STARTUP_RECOVERY.md)核对自有进程和文件访问状态，避免反复启动同一失效环境 |
| 大厅连接失败 | 核对本地服务、证书有效期和配置恢复状态；保留具体错误与本次日志 |
| 选图后没有 Bot 或无法准备 | 确认该地图和模式已准备、人数符合入口容量，等待名单完成同步 |
| 人物满级但配件仍锁定 | 检查武器等级与具体配件门槛；人物等级和全物品所有权分别处理 |

图形启动器的日志区域显示本次状态；命令行也会输出进度。原生启动记录位于对应配置的 `state_dir` 下，默认在 `launcher/state/native/`；脚本实验记录位于 `server/research-output/gamehub/`。

多人共享对局已有核心验收，但常规启动器每个会话仍自行管理本地后端，双端需要独立目录、身份和共同服务配置。准备联机时参照[多人验收](MULTICLIENT_ACCEPTANCE.md)，远端部署参照[Linux 联调](REMOTE_LINUX_ACCEPTANCE.md)；仅填写“连接转发”的服务器地址不能完成整个 UDP 游戏链路。
