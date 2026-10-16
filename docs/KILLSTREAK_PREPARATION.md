# 连杀资源、背包与调用契约

目标为固定3.3.3.3客户端与纯Rust服务端。本版资源、背包、原生HUD、权威进度、循环发奖、稳定库存和部分效果已接入；普通开放与原生验收逐项见[连杀覆盖](KILLSTREAK_COVERAGE.md)。fixture中的整体 `gameplay_verified=false` 不否定已验证单项，也不代表全部奖励完成。

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

本版空投 C4=5、AC130 C4=12、遥控炸弹 C4=25；不能按其它 COD 版本的印象改成4、11或3。C5在部分条目有7／10／18等值，其选择条件和服务端规则尚未恢复，准备数据保留 `column5_raw`，不当作升级门槛。C13保留为 `weapon_token_c13`，例如 UAV 的 `killstreak_uav_mp`；各奖励的注册、动作与效果范围须逐项验证。

Alpha/LAN 的 原生状态采样 和相关 GSC 仅提供调查线索，其69×24目录不属于本版。没有复制参考脚本实现或替换客户端资源。完整原始表、HUD表达式和反汇编留在忽略目录；[最小准备数据](../fixtures/protocol/killstreaks-3.3.3.3.json)只保留必要标识、数值、布局与来源摘要。

## 背包与身份

本版 `mp/playerdata.def` v233 的60个 `weaponClasses` 每项都有四个 Int32 `killstreaks[0..3]`。偏移逐叶读取，不按固定步长推算。第0项为4402／4406／4410／4414；第59项为12993／12997／13001／13005，均相对已验证的 public snapshot 起点。

沿用 `game::player::class_selection::global_class_index` 的当前角色×5＋角色内背包槽，再取该项四个真实偏移。通用物品读取器 `0x593040` 在正整数分支返回 `raw-1`，0／1解析为空。预览解析使用经CRC／投影校验的档案、当前角色背包和所有权，再通过kis商城条目的精确匹配获取原生ID；三项按成本排序。硬汉对普通预览成本减1，不改变额外核弹25杀门槛。未匹配Pro条目不映射基础奖励，也不获得可用动作。第四个字段存在不代表普通背包始终有四个可用奖励槽。

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

普通三项仅在本轮恰好到达各自门槛时逐项发放，不在较晚节点补发被跳过的奖励。3/5/7分别在第3/5/7、10/12/14、17/19/21杀取得；后续轮次同样处理。

普通三项按最高门槛循环：3/4/5在第3/4/5杀发奖，第6杀进入新一轮，第8/9/10杀再次达到门槛。调用奖励不会重置轮内进度，同种奖励最多持有一份。技能直接击杀和其连锁爆炸只计战绩，不推动奖励计数或额外25杀核弹；实际最佳连杀记录与奖励计数分开维护。natural-cycle-native-result.json记录零起始普通近战完成两轮、第一轮侦察机消耗和第8杀重新取得、跨轮反侦察机只保留一份。reward-exclusion-native-result.json记录29次攻击直升机新增击杀不改变进度9且不发核弹。连锁来源另有权威回归覆盖。

只读HUD捕获 `hud-probe/hud-assets.json` 的连接状态为10，确认已加载 `killstreaks_hd2`（124项）、`killstreak_pickup`（3项）、`killstreak_hint`（1项）和 `killstreak`（3项）。实际表达式确认：

- `killstreaks_hd2`：查本版奖励表C0原生ID，取C15图标。
- `killstreak`：查C1名字，取C15图标。
- `killstreak_hint`：查C1名字，取C17图标。
- 主HUD引用五个 `+actionslot 1..5` 绑定，以及 `killstreak`、五个ID和部分目标查询。

`killstreaks_hd2` 的普通三项使用ID1..3、动作槽2..4。额外奖励使用ID0、动作槽1；条目90..95按奖励表C15查图标。ID4／动作槽5是独立王牌槽，条目120..123还要求档案的 `acestreak>0`，图标由专用getter提供；不能把额外核弹放到此处后仅依靠ID15显示。本版键位图为4..8使用连杀奖励，E近身攻击。字段入PS仍不等于画面和键盘调用已验收。

`codol_protocol::game::channel::usercmd::UserCommand` 已解析 `buttons`、三组 `selected` 和两组 `remote`。现有0x20000／0x200000控制的是对应附加字段的存在；不能直接认定它们就是“使用连杀”。各奖励的具体输入、事件及切回主枪须按对应原生验收确认。奖励武器还需同时满足11位快照ID和当前10位usercmd武器字段的编码限制。

## 服务端运行边界

实际模块为 `game/rewards` 的进度、稳定库存和控制器，`game/modes/tdm` 负责权威击杀与效果编排，`transport/game/connection` 负责输入／资源ACK与复制。协议写入／清理PS和事件，回放保存世界而不重复发奖；详细归属见[模块结构](SERVER_MODULARIZATION_PLAN.md)。

一次死亡按受害者及死亡序号去重。奖励进度、25杀核弹与战绩分开，奖励直接／连锁击杀不推进奖励门槛。实例先预留，核对存活、原生命、资源和环境，再按控制器的提交点消费；失败、取消、死亡／EMP／结束各自有清理路径。当前规则和原生证据见[运行时](KILLSTREAK_IMPLEMENTATION_LOG.md)。

硬汉减少普通门槛，不改变核弹25杀。盲眼、反侦察免疫和EMP保护按已接消费者处理；能力定义存在不代表全部支援都已消费。`modExtraKillstreak`、助攻、持续时长及Pro／王牌规则仍需真实绑定，不自动套用参考表。

## 特殊25杀核弹的运行时边界

`game/rewards/killstreaks.rs` 在权威敌方死亡结算中推进本条命进度，按死亡序列去重，死亡清零进度并保留已获得核弹，存活重生后恢复。采用本地Alpha／LAN兼容规则：固定25杀、每条命最多授予一次、最多持有一个待用核弹；调用需要存活、拥有奖励、资源启用且没有进行中的核弹。调用后10秒触发闪光／爆炸，1.5秒后原子结算全部存活参与者并强制调用方获胜。调用方死亡不取消倒计时。这些时序与规则是兼容行为取舍，不是恢复出的本版官方服务端脚本。

阿富汗地图的只读资源准备确认武器323／伤害武器334、切出1000ms／举起833ms、闪光77／爆炸76／余波61、计时声1986／音乐1976、获得文本448／调用文本556。`tools/prepare_nuke.py` 核对客户端摘要、连接状态、初始化CRC、武器注册和FX／声音／文本唯一编号，输出仅包含必要资源元数据。地图通过 `nuke_file` 显式启用；未准备相应资源的地图不发布核弹。

Rust启动器的阿富汗实机验收已确认：从0进度完成25次敌方权威死亡，24时无核弹，25时发放ID15／动作槽1／武器323及原生图标。当前客户端绑定显示7，普通按7成功切入323、消耗一次奖励，约833ms后返回原武器并清掉奖励库存；原生10秒倒计时可见，FX环确认闪光、爆炸和子冲击波。11.5秒后服务端结算全部存活参与者，调用方获胜，客户端返回12人原生结算。三项背包图标和进度格也已实机可见；UAV已接入，其余覆盖状态见 [KILLSTREAK_COVERAGE.md](KILLSTREAK_COVERAGE.md)。

调用公告使用本版e消息及0x14本地化／0x15字面参数段，正确展开调用者名字；裸HUD文本槽不能替代带参数的公告。原生视觉确认调用公告无残留 `&&1`。核弹视觉链路已接入：原生里程碑采样和图片分别确认`mpnuke`强闪白及`mpnuke_aftermath`褐色余波／结算画面。完整慢动作、独立听感及余波61的实际创建仍需分别核验；不把FX创建状态视为完整画面表现。证据在忽略目录 原生状态采样，自然25杀记录与24起始进度的消息回归分别保存。

隔离验收可配置 `acceptance_stationary=true` 让六个敌人集中站定、不攻击，保留正常权威伤害和死亡后3秒重生；自然门槛验收使用 `acceptance_seed=0`。这些设置同时要求资源目录与服务数据根目录包含 `.codol-nuke-acceptance`。正常资源不带诊断选项；种子必须低于25，不能用于自然击杀验收。客户端主程序保持未修改，采样只读，按键和鼠标来自普通窗口输入。

## 验证与重现

准备器拒绝错误客户端摘要、非只读采集、参考版本24列表、残缺单元格、重复原生／商城ID、投影或重叠背包字段以及超出目标5位范围的成本。五项定向回归覆盖编号域、稀疏ID50、Pro未匹配、60项四槽偏移和错误来源；全部121项Python检查通过。独立C#探针构建通过，并从运行中的标记副本只读采集表和HUD。Rust格式、Clippy、协议／服务端和集成测试通过；原生HUD表达式支持当前槽位映射。原生右侧显示、自然25杀、核弹调用、FX创建及结算已通过上述范围验收。

在 `server/` 下重现准备数据；输出采用独占创建，不覆盖已有文件：

```sh
python3 tools/client_state.py --build --prefix-name PROBE_PREFIX
python3 tools/client_state.py --experiment <运行中的experiment.json> --assets
python3 tools/client_state.py --experiment <运行中的experiment.json> --hud-assets
.venv/bin/python tools/prepare_killstreaks.py \
  --assets research-output/killstreak-preparation/client-assets.json \
  --client ../CODOL/codoMP_client_shipRetail.exe \
  --hud-assets research-output/killstreak-preparation/hud-probe/hud-assets.json \
  --output research-output/killstreak-preparation/reproduced.json
python3 -m unittest discover -s tools/tests -v
```

前两种采集命令返回新文件路径；采集新证据时将准备命令的输入替换为这些路径。编译prefix与游戏prefix分开，当前准备采集使用独立命名的探针，原始源码快照及构建摘要留在 `killstreak-preparation/`。启动游戏使用Rust 启动器，沿用标记副本、隔离账号和只读采样约束。

后续验收至少包括：未达标／刚达标／多次达标；同一死亡重复提交；死亡／自杀／坠落／换背包；敌我阵营及技能成本／抗性；多个已获得实例；连续按键／重发／失败调用；资源ACK前后；delta从非零清零；历史回放；最终击杀与结算；退出后重开及第二张地图。先用合成事件和独立字段解码检查逻辑，再以隔离账号、真实输入、只读PS／HUD和效果状态采样确认本版行为；视觉／听感集中复核，不把状态位或提示出现当作效果完成。
