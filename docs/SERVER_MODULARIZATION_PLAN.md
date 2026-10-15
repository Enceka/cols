# 服务端模块结构与扩展约束

本文件描述 `server/` 中纯 Rust、CODOL 3.3.3.3 后端的当前模块结构。工程保留 `codol-server`、`codol-protocol` 两个 crate，进程、CLI 参数、账号文件路径和协议版本保持原有契约。

## 目录与职责

```text
crates/server/src/
├── main.rs                         # Tokio 入口，调用 app::run
├── lib.rs                          # 顶层模块入口
├── app/
│   ├── cli.rs                      # CLI 解析、命令执行与启动装配
│   ├── readiness.rs                # 原生启动器的进程身份与就绪契约
│   ├── config.rs
│   └── services/                   # LocalServices：业务组合、房间投递与档案快照
├── contracts/                     # 无凭据的 AccountId、接收者身份与任务结果
├── transport/
│   ├── auth/                       # AUTH 的 HTTP/TLS 接入
│   ├── lobby/
│   │   ├── server/                 # TCP、加密、FIFO、连接生命周期
│   │   ├── router.rs               # 支持的 service/task 编号分发
│   │   ├── handlers/               # 档案、商城、文件的协议适配
│   │   └── delivery.rs             # 有界邮箱、投递与断线清理
│   ├── game/
│   │   ├── server/                 # UDP 调度、准入验证与状态准备选择
│   │   ├── handshake.rs
│   │   └── connection/
│   │       ├── mod.rs              # Connection、同步状态与入口
│   │       ├── channel.rs          # 原生握手、命令解码与连接阶段
│   │       ├── replication.rs      # 快照、delta 与 ACK 基线
│   │       ├── shared.rs           # 共享真人对局的输入与复制桥接
│   │       ├── latency.rs          # native 序号／ACK RTT采样与发布
│   │       ├── loadout.rs          # 配装确认、开局宽限与下次出生应用
│   │       ├── input.rs            # 输入队列与当前局内行为桥接
│   │       ├── matchplay.rs        # 训练/TDM 推进与重生桥接
│   │       ├── world.rs            # 投掷物、动态物件的局内桥接
│   │       ├── rewards.rs          # 奖励调用与核弹流程桥接
│   │       ├── progression.rs      # 完局经验提交与可靠提醒
│   │       ├── weapon_drops.rs     # 掉枪、提示、拾取与资源确认
│   │       └── presentation.rs    # 提示、语音、击杀和结算表现
│   ├── stun/
│   └── http.rs
├── backend/
│   ├── identity/session.rs         # 登录会话、凭据有效性
│   ├── profile/                    # 公私有档案读取与保存
│   ├── inventory/                  # 基础物品所有权
│   ├── progression/                # 等级模型、经验事务、结算回执与读取投影
│   ├── commerce/                   # 商品、钱包、永久物品购买
│   ├── publisher/                  # 公共文件、元数据、playlist 版本
│   ├── files/                      # 玩家文件、配额与身份范围
│   └── rooms/
│       ├── mod.rs                  # 房间、成员、准备与名单
│       ├── directory.rs            # 浏览快照、分页与成员操作
│       ├── bots.rs                 # 原生房间 Bot 展示策略
│       └── admission/              # 分配、预订、开局校验与租约
├── game/
│   ├── session/
│   │   ├── local_world.rs          # 单真人兼容世界的权威可变状态
│   │   ├── shared/                 # 房间共享世界、真人输入、Bot与投掷物
│   │   ├── weapon_registry.rs      # 对局资源索引分配
│   │   ├── catalog.rs              # 已准备的地图/模式与 playlist 绑定
│   │   └── prepared/               # schema、加载验证、配装、实体与报文装配
│   ├── world/
│   │   ├── collision/              # 几何查询、玻璃与动态物件碰撞
│   │   ├── props.rs
│   │   ├── weapon_drops.rs         # 权威地面枪与弹药交换
│   │   └── math.rs
│   ├── player/                     # 移动、冲刺、落地、生命与背包选择
│   ├── weapons/
│   │   ├── held/                   # HeldWeapons：持枪状态机
│   │   │   ├── parameters.rs       # 武器动作参数
│   │   │   ├── ammunition.rs       # 弹药与补给
│   │   │   ├── actions.rs          # 切枪、射击、ADS、冲刺
│   │   │   ├── reload.rs
│   │   │   ├── offhand.rs
│   │   │   ├── aim.rs
│   │   │   └── rewards.rs
│   │   ├── grenade/
│   │   └── smoke.rs
│   ├── combat/                     # 命中、伤害、命中区域
│   ├── progression/                # 玩法击杀去重与可信经验输入
│   ├── ai/                         # 控制、导航、装备、动画
│   ├── status/                     # 技能、闪光/震撼、毒气、EMP
│   ├── rewards/                    # 循环进度、稳定库存、雷达／EMP、部署、空投、选点及远控
│   ├── presentation/               # 语音、反馈、回放、结算展示
│   └── modes/tdm/
│       ├── config.rs               # 模式、准备／宽限、出生、名单、计分与时限配置
│       ├── spawns.rs               # 固定基地与战术共享点池选点
│       ├── rules.rs                # 计分终局、计时与胜负规则
│       ├── mod.rs                  # TdmMatch 及模式生命周期
│       ├── bots.rs                 # TDM 中的 Bot 调度
│       ├── combat.rs               # TDM 击杀归因与战斗编排
│       ├── aircraft.rs             # 攻击飞行器与模式交互
│       ├── location_strikes.rs     # 选点、范围判伤与爆炸
│       ├── remote.rs               # 远控生命周期与原生命归属
│       └── rewards.rs              # TDM 奖励与世界对象编排
├── content/
│   ├── maps/                       # 原生地图标识、开局选择解析、小地图定义
│   ├── weapons/                    # 武器资源、配件组合与属性
│   └── characters/                 # 外观资源定义
├── infra/
│   ├── storage/                    # 有界读取、原子私有写入、账号路径与写锁
│   └── capture.rs                  # 抓包与观测输出
├── admin/                          # Web 管理入口
└── diagnostics/                    # observe、地图验证、出生准备、离线账号维护
```

`codol-protocol/src/` 按以下协议领域组织：

- `auth/`：AUTH 请求、票据和 bootstrap 配置。
- `lobby/`：大厅通道、任务编码、公私有档案、公共文件和购买报文。
- `room/`：房间描述、浏览、成员消息和剧情分配格式。
- `game/channel/`：加密 records、wire、usercmd。
- `game/gamestate.rs`、`game/snapshot/`：gamestate、玩家与实体字段、delta、事件和表现报文。
- `discovery/stun.rs`、`tls.rs`：地址发现与 TLS 观察解析。

使用标准 Rust 目录模块；内部较大的实现按职责拆为同一私有类型的子模块。测试随模块放置，跨模块传输与回放测试保留在 `crates/server/tests/`。

## 实际接口与数据所有权

| 边界 | 入口与约束 |
| --- | --- |
| 应用启动 | [`app::run`](../crates/server/src/app/mod.rs)，二进制入口仅建立 Tokio runtime |
| 启动器就绪 | `app/readiness` 在端点、地图和业务装配完成后原子发布本次进程记录；接口见 [NATIVE_LAUNCHER.md](NATIVE_LAUNCHER.md) |
| 业务组合 | [`LocalServices`](../crates/server/src/app/services/mod.rs)，组合各业务服务和房间投递；本身不再包含全部任务处理与存储实现 |
| 大厅分发 | [`TaskRouter`](../crates/server/src/transport/lobby/router.rs)，分发到按业务组织的 handlers；handlers 保持原生错误码、回执与顺序 |
| 账号数据 | [`AccountStore`](../crates/server/src/infra/storage/accounts.rs)，一个实例共享同一数据目录与写锁，不能为各业务单独建立互不相关的写锁 |
| 购买 | [`CommerceService`](../crates/server/src/backend/commerce/service.rs)，余额与购买授予在同一个钱包文件中原子提交；库存查询合并基础物品与购买授予 |
| 房间准入 | [`GameAdmissions`](../crates/server/src/backend/rooms/admission/mod.rs)，独立于 UDP listener；注册表内部私有，向接入层返回已验证的分配快照 |
| 本地世界 | [`LocalWorld`](../crates/server/src/game/session/local_world.rs)，持有准备状态、玩家、移动、生命、Bot/TDM、投掷物与状态效果 |
| 连接同步 | [`Connection`](../crates/server/src/transport/game/connection/mod.rs)，持有一份 `LocalWorld` 以及该连接的 ACK、可靠命令、输入基线、快照历史与表现状态 |
| 模式规则 | [`tdm::Rules`](../crates/server/src/game/modes/tdm/rules.rs)，不访问网络或实体，负责现有 TDM 的计时和胜负判断 |
| 碰撞 | [`Collision`](../crates/server/src/game/world/collision/mod.rs)，静态几何共享；`fresh_session` 创建独立玻璃/物件状态，配装切换保留当前世界碰撞 |

`LocalWorld::from_prepared` 为每个世界建立独立的动态碰撞状态；直接构造世界也保持玻璃与物件隔离，不依赖连接层额外处理。其可变组件仅在 crate 内可见，当前由连接桥接代码协调；它不是供账号业务修改的公共上下文。账号文件、房间成员、网络凭据不能加入该对象。

存储和账号业务使用 `contracts::account::AccountId`，不依赖网络会话结构，也不为读取档案构造虚假的登录凭据。认证层将已验证会话转换成账号身份；`contracts` 不反向依赖业务实现。

等级业务通过 `contracts::progression::MatchAward`接收可信对局结果，事务和回执位于`backend/progression`，由`app/services`组合。`game/progression`收集已确认的击杀及生效武器物品ID，接入边界提交完成对局并投递K／库存XP／提示命令，可靠ACK确认原回执。策略由显式本地配置提供，传输层不推导经验倍率，玩法不直接修改账号文件。具体启用范围和验证见[等级接入进展](PROGRESSION_PREPARATION.md)。

已准备地图目录会加载并组合运行时准备状态，因此位于 `game/session/catalog`；出生准备工具会执行碰撞与模式校验，因此位于 `diagnostics/map_starts`。`content` 只承载资源定义和静态选择解析，不反向构造运行时世界。

## 依赖和扩展规则

1. `backend` 不依赖 `transport`、`app` 或 `admin`。协议边界共享类型放 `contracts`；房间分配由房间业务模块拥有。
2. `game` 不依赖账号业务和接入服务；所需内容来自 `content`，原生格式依赖 `codol-protocol`。账号配装数据在边界传入。
3. `content` 不依赖 `game`、`backend` 或网络/应用模块。运行时装配与离线验证放到对应模块。
4. `app` 负责组合业务服务与接入流程；网络层通过组合入口或具体服务接口调用。不要把新业务加入中央 `LocalServices::handle` 的大 match。
5. 模块内部默认私有，通过 `mod.rs` 选择性导出。私有实现子模块使用 `pub(super)` 共享必要方法；不为文件移动统一公开所有类型。
6. `main.rs`、`lib.rs` 是服务端 `src` 根目录仅有的 Rust 文件。新功能进入所属模块，不恢复 `game_*.rs` 平铺布局，也不建立无明确归属的 `common`/`utils` 大文件。
7. 嵌入 fixture、配置和管理页面的路径使用 `CARGO_MANIFEST_DIR`，避免跟随源文件层级改变。Python 工具、运行数据目录和启动器参数保留原有入口。

新增枪械行为进入 `game/weapons/held`，资源解释进入 `content/weapons`；新状态效果进入 `game/status`；新奖励的实体能力进入 `game/rewards`，其模式计分与胜负影响由模式层处理。

新增模式应先明确其配置、开局条件、事件处理、出生策略和结果。仅有 TDM 独立规则时不预建空 trait、插件加载器或未实现的模式模块。剧情的 `coop` 分配/开局校验目前属于 `backend/rooms/admission` 的显式采集路径；正式任务运行实现时才增加 `game/modes/story/`。

## 保留的运行边界

支持的多人房间由 `game/session/shared` 持有共享权威世界，游戏接入绑定各真人准入与接收者映射；每个连接保留独立加密、输入、ACK和复制历史。单真人兼容路径仍使用 `LocalWorld`。核心双客户端及Linux成品联调已通过，具体边界见[多人验收](MULTICLIENT_ACCEPTANCE.md)。

模拟与原生 ACK、资源初始化、输入消费顺序仍在 `transport/game/connection` 的桥接模块中协调。`PreparedGameState` 仍包含运行时构造和本版报文投影，因此留在 `game/session/prepared`，没有将其标为纯只读内容。Bot/奖励的通用实现位于共用玩法模块，TDM 模式目录内仍保留对这些系统的编排。

桥接中的模拟、ACK与资源顺序仍需继续收紧，通过输入／输出事件对接复制层；不能把单连接ACK当成整局时钟。共享世界保证单次推进；中途背包、个人回放、经验、慢连接和完整重连仍按各自边界验证。

目录边界本身不改变对局时钟、协议、ACK/重发、账号 schema 和购买一致性；配装按准备／未交战宽限即时应用，其它活动条命在出生时应用，见 `MATCH_FLOW_AND_SPAWNS.md`。不通过目录拆分启用尚未支持的剧情、模式或装备。

## 检查与验收

```sh
python3 tools/check_architecture.py
sh tools/cargo.sh fmt --all -- --check
sh tools/cargo.sh test --workspace --locked
python3 -m unittest discover -s tools/tests -v
sh tools/cargo.sh run --locked -p codol-server -- check-config
sh tools/check.sh
```

`check.sh` 运行目录/直接依赖检查、fmt、Clippy、Rust workspace 测试和 Python 测试。架构检查是源码约束检查，不能替代 Rust 类型检查或审查宏、别名产生的依赖。

重点回归约束：

- 档案保存、重启读回、购买并发、扣款/授予原子性与身份隔离。
- 房间查询、原生成员消息、会话替换、准入与租约。
- 输入 ACK 基线、分片/重发、时钟推进、重生朝向、配装延迟生效。
- 不同本地世界的玻璃/物件隔离；同一世界配装变化不重置破坏状态。
- 移动与碰撞、武器状态机、Bot、TDM 计分/时限/核弹与返房流程。

现有 Rust 单元/集成测试通过不等于新增玩法已完成原生客户端验收。带 `ignored` 的资源/实机诊断应按各自文档单独运行，不能改为无条件跳过以掩盖回归。
