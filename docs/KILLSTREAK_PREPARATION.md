# 连杀奖励接入准备

目标为未修改的 3.3.3.3 客户端和纯 Rust 服务端。当前完成本版资源目录、背包布局、原生 HUD／动作槽的静态协议依据及服务端接入方案；运行时尚未发放或使用连杀奖励。`gameplay_verified=false` 表示没有完成达标、调用及效果验收。

## 本版资源

只读采集 `research-output/killstreak-preparation/client-assets.json` 中实际加载的 `mp/coKillstreakStatsTable.csv` 为 **29 行、33 列**，`mp/coMarketplaceItems.csv` 有 **44 个 kis 条目**。按完整名字精确匹配得到23个对应项；不能用商城物品 ID、物理行号或参考版本 ID 代替本版奖励 ID。

下表的 C4 为本版原生函数 `0x6c1270` 按名字查询的基础成本值。它是制定击杀门槛的依据，尚不包含模式、技能及其它成本分支。同名匹配不证明大厅可选、完整效果资源已加载或玩法可用。

| 原生 ID | 本版名字 | 商城物品 ID | C4 |
| ---: | --- | ---: | ---: |
| 1 | uav | 60076 | 3 |
| 2 | airdrop | 60077 | 5 |
| 3 | counter_uav | 60078 | 4 |
| 4 | airdrop_sentry_minigun | 60079 | 5 |
| 5 | predator_missile | 60080 | 5 |
| 6 | precision_airstrike | 60081 | 6 |
| 7 | harrier_airstrike | 60082 | 7 |
| 8 | helicopter | 60083 | 7 |
| 9 | airdrop_mega | 60084 | 8 |
| 10 | helicopter_flares | 60085 | 9 |
| 11 | stealth_airstrike | 60086 | 9 |
| 12 | helicopter_minigun | 60087 | 11 |
| 13 | ac130 | 60088 | 12 |
| 14 | emp | 60089 | 15 |
| 16 | sentry | 60090 | 5 |
| 17 | mortar_mp | 60091 | 6 |
| 18 | rcbomb_mp | 60092 | 25 |
| 19 | sr71_mp | 60093 | 8 |
| 20 | dogs_mp | 60094 | 10 |
| 21 | sam_turret | 60096 | 4 |
| 22 | airdrop_sam_turret | 60095 | 4 |
| 23 | juggernaut | 60131 | 11 |
| 26 | missile_drone | 60138 | 4 |

还存在 `none`（ID0），以及未匹配到 kis 商城条目的 `nuke`（15）、`juggernaut_kingpin`（24）、`locker`（25）、`consumable`（27）、`super_katana`（50）。最后一条的物理行号是28，原生 ID 是50，遍历不能遇到缺号就停止。零成本条目也不能自动进入“零杀即发奖”的普通连杀流程。

商城中的21个 Pro 条目（60097..60117）在这份本版加载表中没有同名奖励行。其身份保留为未匹配；没有把 `_pro` 去掉后映射基础奖励。需继续调查其它资源表、技能变体或模式路径，不能以参考服有 Pro 行为证明本版实现。

本版空投 C4=5、AC130 C4=12、遥控炸弹 C4=25；不能按其它 COD 版本的印象改成4、11或3。C5在部分条目有7／10／18等值，其选择条件和服务端规则尚未恢复，准备数据保留 `column5_raw`，不当作升级门槛。C13保留为 `weapon_token_c13`，例如 UAV 的 `killstreak_uav_mp`；完整武器注册、动画、消耗与启动链仍需验证。

Alpha/LAN 的 `dedicated/docs/CODOL_KILLSTREAK_CATALOGUE_20260914.md` 和相关 GSC 仅提供调查线索，其69×24目录不属于本版。没有复制参考脚本实现或替换客户端资源。完整原始表、HUD表达式和反汇编留在忽略目录；[最小准备数据](../fixtures/protocol/killstreaks-3.3.3.3.json)只保留必要标识、数值、布局与来源摘要。

## 背包与身份

本版 `mp/playerdata.def` v233 的60个 `weaponClasses` 每项都有四个 Int32 `killstreaks[0..3]`。偏移逐叶读取，不按固定步长推算。第0项为4402／4406／4410／4414；第59项为12993／12997／13001／13005，均相对已验证的 public snapshot 起点。

沿用 `game_class_selection::global_class_index` 的当前角色×5＋角色内背包槽，再取该项四个真实偏移。通用物品读取器 `0x593040` 在正整数分支返回 `raw-1`，0／1解析为空；连杀选择调用链及当前背包读数还需定向复核。实现时校验已验证档案的CRC、投影、所有权、kis类别、完整名字匹配及实际处理器能力。第四个字段存在不代表普通背包始终有四个可用奖励槽。

四个档案选择字段、五个HUD预览字段和五个动作槽是不同数组。原生奖励 ID1..50、商城物品 ID60076..60138、动态武器注册 ID及NCS材质编号也属于不同编号域。

## 已确认的协议消费者

所有地址为本版 PE 的首选镜像VA，PS偏移为 playerState 相对偏移。主程序SHA-256为 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。准备器核对整份235项字段表及选定消费者的代码摘要，不执行客户端函数。

| 数据 | PS偏移 | 本版字段编号 | 位宽 | 消费者 |
| --- | --- | --- | ---: | --- |
| 当前进度 | 0x64 | 88 | 8 | `0x519820` |
| 奖励ID0..4 | 0x68..0x78 | 126..130 | 8 | `0x519940` |
| 目标值0..4 | 0x7c..0x8c | 131..135 | 5 | `0x519980` |
| 动作类型0..4 | 0x560..0x570 | 139、187、100、174、222 | 2 | `0x4327d0`、`0x55d510` |
| 动作武器0..4 | 0x574..0x584 | 144、203、141、173、221 | 11 | 同上 |

HUD查询 `killstreak`／`killstreakID0..4`／`killstreakTarget0..4` 通过 `0x56f2e8` 邻近的名字分派到这些getter。`0x519860` 在有ID的槽位中选择高于当前进度的最小目标；达到全部目标后返回已配置的最大目标。字段必须独立发送，不能将累计击杀数或记分板字段当作奖励库存。

`0x55d510` 的动作类型1分支读取对应武器ID，并检查真实PS武器库存后请求切换。仅发送奖励ID或动作类型不能构成可使用的奖励；需要注册武器、库存、输入base、预测武器状态和资源ACK共同闭合。参考服的动作槽PS+0x544／+0x558布局与本版不同，本版使用上表0x560／0x574。

只读HUD捕获 `hud-probe/hud-assets.json` 的连接状态为10，确认已加载 `killstreaks_hd2`（124项）、`killstreak_pickup`（3项）、`killstreak_hint`（1项）和 `killstreak`（3项）。实际表达式确认：

- `killstreaks_hd2`：查本版奖励表C0原生ID，取C15图标。
- `killstreak`：查C1名字，取C15图标。
- `killstreak_hint`：查C1名字，取C17图标。
- 主HUD引用五个 `+actionslot 1..5` 绑定，以及 `killstreak`、五个ID和部分目标查询。

这些是资源和消费者证据；没有向客户端授奖，不代表获得提示、动作图标、键盘操作或效果已验收。动作图标覆盖、可用性旗标、选择顺序、中央提示、声音／播报的网络生产者仍需恢复。

`game_usercmd::UserCommand` 已解析 `buttons`、三组 `selected` 和两组 `remote`。现有0x20000／0x200000控制的是对应附加字段的存在；不能直接认定它们就是“使用连杀”。动作键到具体按钮／选中索引的映射、使用武器事件和切回主枪须在本版实采。奖励武器还需同时满足11位快照ID和当前10位usercmd武器字段的编码限制。

## 服务端接入方案

以下是待实施方案，运行时尚无相应模块。

| 位置 | 接入职责 |
| --- | --- |
| `tools/prepare_killstreaks.py` | 由只读表／schema和本版原程序生成最小准备数据；已具备 |
| `game_loadout.rs`、`game_state.rs` | 解析当前角色背包选择和所有权；冻结本条命配置；经实际资源准备注册奖励武器 |
| 新 `game_killstreaks.rs` | 每名参与者的进度、已获得队列、选择项、使用状态、唯一奖励实例及生命周期；能力和成本策略独立于表现 |
| `game_match.rs` | 在权威死亡结算 `credit`／`actor_fell` 统一处理攻击者进度和受害者死亡；玩家／Bot共用规则 |
| `game_connection.rs` | 资源ACK后发布可用动作；消费经过验证的本版输入；派发获得／使用提示；保留换背包与下次出生流程 |
| 新 `protocol/game_killstreaks.rs`、`game_snapshot.rs` | 按上表写入／清除PS字段；以ACK确认帧作delta基线；明确超界拒绝及零值清除 |
| `game_replay.rs` | 历史帧携带当时的HUD／库存／动作；回放不能重复授奖或调用效果 |
| 各奖励效果模块 | 根据效果拥有者和阵营执行雷达、干扰、实体、导航、判伤及终止；资源编号按各图准备 |

结算建议使用 `(对局ID, 受害者ID, 死亡序列)` 标识一次真实死亡；仅在权威判伤使生命降到零时消费。`drain_match_kills` 负责击杀栏、尸体、拾荒和回放表现，不作为第二处独立计数入口。死亡后和结算后的重复输入、重发或历史帧都不能再次发奖。

建议状态分别持有本条命进度、本局最高进度、背包所选奖励、已经获得但未使用的实例，以及正在执行的支援。当前 `ScoreboardRow.killstreak` 只有协议字段，尚无进度生产者；其生涯／本局统计语义需另查，不能据此承载上述全部状态。现有 `Kill` 仅有攻击者、武器、爆头／近战／坠落等信息；效果接入时补充击杀来源和支援拥有者，才能执行奖励击杀是否继续累计的规则。

使用流程建议为：收到有效输入→核验存活／对局阶段／所有权／实例→检查效果资源与环境→建立效果→消耗一次实例→更新库存与HUD。失败请求的保留、切枪中断和并发效果策略先按本版行为核实；不得返回成功后才发现没有实际效果。

## 行为取舍与实现顺序

仍需恢复的规则包括：死亡是否保留已获得奖励；连杀奖励造成的击杀是否继续累计；同一条命是否循环发奖；奖励队列排序和上限；换背包后如何处理已有库存；第四槽／额外奖励的门槛；同类支援重叠和失败调用的消耗。它们不由本版表的存在或参考GSC自动确定。环境坠落、自杀、队友受击和对局结算必须走明确分支，避免共享攻击者索引造成误发。

已有本版能力表提供硬汉 `specialty_hardline`／60042 的 `modKillstreakShift=-1`；盲眼60043具有 `modNoSentryGuns`、`modNoHelicopter`、`modNoAirplane`；忍者60075与硬链接60213具有 `modInvalidCounterUav`，硬链接还具有 `modEmpProtectionScale=0`。这些能力已在背包聚合，不等于连杀消费者已实现。`modExtraKillstreak`、`modKillstreakAssists`、`modKillstreakDuration` 等定义仍需关联实际技能／模式和消费公式。

建议先完成进度／发奖／库存／原生HUD／调用和生命周期，再用UAV打通一次完整调用，随后接反侦察机和全局EMP。其余按依赖展开：

| 奖励类别 | 必须补齐的能力 |
| --- | --- |
| UAV／反侦察机／SR71 | 友敌雷达可见性、扫描周期、持续时间、干扰优先级、免疫和隐藏技能；地图底图已存在，雷达行为尚未接入 |
| 全局EMP | 团队支援期限、敌方状态、设备／支援禁用、免疫及恢复；现有个人EMP手雷不能代替全局规则 |
| 空投／哨戒／SAM | 标记弹、运送和箱子实体、真实拾取归属、部署净空、目标筛选、射击／防空及回收 |
| 导弹／空袭／迫击炮 | 位置选择和确认、远控输入／镜头、飞行及爆炸实体、碰撞、队友判伤和伤害归属 |
| 自动／操控直升机、AC130 | 本图空中路线、载具和炮塔网络状态、锁定／防空、座位镜头、武器弹药及退出 |
| 遥控车／军犬／重装／特殊条目 | 独立载具／AI／导航／模型动作或模式规则；不能用屏幕提示代替效果 |

这是依赖顺序建议，不是全部奖励已获实现验收。每项应按实际处理器和资源准备状态显式发布可用性，未匹配Pro或只有表行的奖励不能进入成功调用路径。

## 验证与重现

准备器拒绝错误客户端摘要、非只读采集、参考版本24列表、残缺单元格、重复原生／商城ID、投影或重叠背包字段以及超出目标5位范围的成本。五项定向回归覆盖编号域、稀疏ID50、Pro未匹配、60项四槽偏移和错误来源；全部117项Python检查通过。独立C#探针构建通过，并从运行中的标记副本只读采集表和HUD；没有完成奖励实机玩法验证。

在 `server/` 下重现准备数据；输出采用独占创建，不覆盖已有文件：

```sh
python3 tools/client_state.py --build --prefix-name wine10-probe-roster-20261010
python3 tools/client_state.py --experiment <运行中的experiment.json> --assets
python3 tools/client_state.py --experiment <运行中的experiment.json> --hud-assets
.venv/bin/python tools/prepare_killstreaks.py \
  --assets research-output/killstreak-preparation/client-assets.json \
  --client ../CODOL/codoMP_client_shipRetail.exe \
  --hud-assets research-output/killstreak-preparation/hud-probe/hud-assets.json \
  --output research-output/killstreak-preparation/reproduced.json
python3 -m unittest discover -s tools/tests -v
```

前两种采集命令返回新文件路径；采集新证据时将准备命令的输入替换为这些路径。编译prefix与游戏prefix分开，当前准备采集使用独立命名的探针，原始源码快照及构建摘要留在 `killstreak-preparation/`。启动游戏仍复用项目既有工具和隔离约束。

后续验收至少包括：未达标／刚达标／多次达标；同一死亡重复提交；死亡／自杀／坠落／换背包；敌我阵营及技能成本／抗性；多个已获得实例；连续按键／重发／失败调用；资源ACK前后；delta从非零清零；历史回放；最终击杀与结算；退出后重开及第二张地图。先用合成事件和独立字段解码检查逻辑，再以隔离账号、真实输入、只读PS／HUD和效果状态采样确认本版行为；视觉／听感集中复核，不把状态位或提示出现当作效果完成。
