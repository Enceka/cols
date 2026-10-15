# 原生客户端与 Rust 启动器接口

启动器通过 `codol-server` 的命令行和本次进程的就绪记录管理服务端，不依赖内部 Rust 模块路径。客户端继续使用本版 AUTH、协议210大厅、STUN与UDP v3对局通道。客户端 EXE/DLL 和游戏资源内容保持原样，仅连接 INI 和本地 CA 在会话中临时调整并恢复。

## 启动与就绪契约

地图目录按 `(map, mode)` 绑定原生房间。默认目录包含战术 `war` 和核弹镇、锈迹之地、雪山之巅、高楼冲锋 `cha`；冲锋由原生创建房间列表进入，再用地图箭头选已准备地图。资源、固定基地复活及模式15结算证据见[冲锋团队竞技](CHARGE_MODE.md)。

启动器启动两个自有进程：

- `serve-auth`：使用 `--local-title-id`、`--service-data-dir`、`--lobby-bind`、`--stun-bind`、`--stun-alt`、`--game-bind`、`--game-state`、`--game-map-catalog`、`--native-bot-rooms`、`--continuous-play`。
- `observe --config`：单独监听 IIPS 资源请求，记录请求但不提供游戏资源。

两个命令可传入 `--launcher-ready-file PATH`。不传时保留原命令行行为。启动器为每次会话选择新的外部目录，不复用旧记录。

服务端完成实际端点绑定、地图加载、Bot策略、房间准入和业务服务装配后，原子写入私有 JSON 文件。写入目的文件必须不存在。普通日志的文字、格式和级别不属于就绪协议；无日志输出时仍可产生就绪记录。

两类记录的公共字段：

| 字段 | 含义 |
| --- | --- |
| `schema_version` | 当前为1；不兼容的字段语义变更须提高版本 |
| `pid` | 写入记录的服务端进程 PID |
| `client_sha256` | 服务端协议支持的原生客户端基线，来自 `codol_protocol::CLIENT_SHA256` |
| `kind` | `native_services` 或 `tcp_observer` |

`native_services` 另外包含：

| 字段 | 当前原生启动器校验 |
| --- | --- |
| `lobby_protocol`、`game_protocol` | 分别为210和3 |
| `title_id` | 必须与临时 `dwauth.ini` 和账号目录选择一致 |
| `auth`、`lobby`、`stun`、`stun_alt`、`game` | 实际绑定地址，须与本次配置一致；端口0时报告操作系统分配的实际端口 |
| `game_state_loaded`、`map_catalog_loaded` | 地图准备和目录已成功加载 |
| `native_bot_rooms`、`continuous_play` | 本地人机和连续游玩已配置 |

`tcp_observer` 的 `listeners` 为名称到实际地址的对象；本地启动器要求只有 `resources`，地址与 IIPS 配置一致。

启动器验证记录不超过16 KiB、不是符号链接，版本、服务类型、PID、客户端摘要、端点和本地玩法开关匹配。两个记录均通过且自有进程仍运行后才临时修改连接配置和启动游戏。未知版本、旧版不支持该选项、绑定失败、地图错误或超时均进入清理路径，不以已有 AUTH 监听或日志文字替代完整就绪。

这是本机进程间的启动检查，不是对网络客户端身份的认证。服务端仍独立验证 AUTH票据、大厅会话、房间成员与对局准入、UDP MAC和序号。

## 当前交互归属

| 客户端动作 | 服务端入口及职责 |
| --- | --- |
| HTTPS认证与本地票据 | `transport/auth` 与 `backend/identity/session` |
| 大厅210、RPC顺序、异步房间推送 | `transport/lobby/server`、`router`、`delivery` |
| 档案、库存、购买、玩家文件及公共playlist | `app/services` 组合 `backend/profile`、`inventory`、`commerce`、`files`、`publisher`；共享 `infra/storage/accounts` 写锁 |
| 浏览、建房、入房、成员准备、Bot装配 | `backend/rooms`；原生消息位于 `protocol/room` |
| 游戏端点分配、预订与开局校验 | `backend/rooms/admission`；由应用装配交给大厅和游戏接入 |
| STUN地址发现 | `transport/stun` |
| UDP握手、MAC、输入与快照 | `transport/game/server`、`handshake`、`connection` |
| 地图、Bot、枪械、TDM与结算 | `game/session`、`ai`、`weapons`、`modes/tdm`；账号配装在接入边界传入 |

启动器不自行实现这些业务RPC，不绕过新房间准入，不保存或制造网络会话凭据。正常账号路径和CLI入口沿用现有约定。房间Bot外观来自当前地图的 `match_config.roster[*].room_loadouts`；旧准备缺少该字段时加载仍可成功，但没有对应房间装配，需按[房间Bot角色展示](ROOM_BOT_AVATARS.md)准备。

目录拆分不等于已经实现真人共享对局。房间浏览和成员消息、模拟Bot展示与真人共享世界是不同能力；当前单局限制见[模块结构](SERVER_MODULARIZATION_PLAN.md)。成长和聊天的可用范围以相应业务文档与验收为准，不由启动器声明启用。

## 验证

```sh
sh tools/cargo.sh test --locked -p codol-server --test launcher_readiness
sh tools/cargo.sh test --locked -p codol-server --test auth_transport --test lobby_transport --test stun_transport
sh tools/cargo.sh test --locked -p codol-server --lib app::services
sh tools/cargo.sh test --locked -p codol-server --lib backend::rooms
python3 tools/check_architecture.py
```

启动契约测试直接运行真实二进制，使用合成地图和临时证书，在关闭日志时核对实际地址及进程身份；错误地图目录不得产生成功记录。启动器回归覆盖不匹配PID、版本、端点及仅有日志时拒绝启动，并检查客户端配置未被修改。完整原生画面和操作仍需独立实机验证。

本机macOS/Wine10样本已通过该就绪契约完成认证、大厅6772项库存、12人Bot房间及原生角色/武器装配、阿富汗地图进入、移动、射击、换弹和结算。会话结束恢复INI/CA，受检文件无差异，独立诊断期间正常账号资料摘要一致。该验证不覆盖Windows、公网联机、真人共享世界或全部新增业务。
