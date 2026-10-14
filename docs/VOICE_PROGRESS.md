# 原生开局播报与角色语音

目标为未修改的 3.3.3.3 客户端与纯 Rust 服务端，EXE SHA-256 为 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。服务端可通过地图准备中的 `voice_file` 启用开局／胜负播报、角色前缀、换弹／投掷喊话、受伤／死亡和延迟击杀喊话。安全屋与坠机之地已通过原生声音通道、实际音频录制和对局流程核对，模式介绍→开战指令的完整顺序和换弹听感获用户确认。普通游玩23份地图准备已启用voice_file，保持7500分／10分钟和原账号配装。资源身份、消费路径和听感验收分别记录。

## 声音传输

下列地址均为本版 EXE 的 VA，不能套用 AlphaBuild 或 2.6.9.7 的地址。

| 用途 | 本版消费路径 | 服务端表示 |
| --- | --- | --- |
| 本地播报／音乐 | `0x57d560` 的 `p` → `0x57b500` → `0x444910` | 可靠字符串 `p <selector>`，恰好一个参数，范围 1..16000 |
| 停止某声音 | `h` → `0x57b560` → `0x4443d0` | 可靠字符串 `h <selector>`，使用同一编号空间 |
| 动态声音名 | `0x44494f`，查 `selector+0xc23` | selector 1..511，配置字符串为 `3107+selector`；0 保留 |
| 原生声音常量 | `0x444919..0x44492c`，减 `0x200` 后查 NCS category5 | selector = 512 + **零起始**常量序号；category5 布局为 `(7058,0,0)`，并非另一个 512 槽配置表 |
| 空间声音 | `0x50e850` 的 EV3 → `0x444910`；EV4 → `0x444970` | EV3 参数为同一 selector，宽度 14bit；事件序号、位置、源实体和接收者过滤由服务器维护 |
| 本地角色自动声音 | `s` → `0x57b690` → `0x4555c0` | `s cg_vo_prefix "UK_F1_"` 等固定前缀，初始化疾跑与狙击呼吸资源 |

第一个原生常量对应 `p 512`，不是 `p 513`。常量播放不占动态槽；当前多人地图动态 NCS 有 8 个名字，已有命中反馈另外注册 `MP_hit_alert` 于相对槽 9／CS3116。后续动态注册必须避开已分配槽。

`0x62dd90` 从 alias list 选择实际变体，记录步长为 `0x6c`。服务端传 alias／selector，由客户端选择样本；具体 WAV 名字不是网络声音编号。

[game_sound.rs](../crates/protocol/src/game/snapshot/sound.rs) 提供 `SoundSelector::constant`／`dynamic`、`play_command`／`stop_command`／`spatial_event` 和 `VoicePrefix`。可靠窗口负责重发；开局和结算各只产生一次命令。角色喊话使用独立的 EV3 临时实体，保留发声者与位置，不向同一声音另发本地命令。

声音实体由 Rust 分配在 1668..1699，弹着范围为 54..1667，投掷物从1700开始；这是服务器的槽位分配，不是客户端规定的固定声音范围。描述表22的field5为14bit声音参数，field9／ES+0x80为11bit源实体。声音实体保留200ms；同一身份在重传／delta中保留原生一次性latch，到期相对已确认基线显式删除，全基线不固定声音事件。

## 资源目录与候选事件

真实已初始化 NCS 捕获按本版排序、去重和 CRC 算法复算，再与各图准备 CRC 核对。目录索引为 `research-output/voice-study/map-catalog.json`：22 张多人地图各有 1494 个原生声音常量、8 个动态常量；竞技场 `za_coliseum` 诊断准备有 1507／24 个。竞技场资源可分析不代表其 PVE 玩法完成。

完整名字、selector、事件目录和来源 SHA-256 在 `research-output/voice-study/maps/<地图>-voices.json`。由用户资源生成的完整目录保留在忽略目录；可提交的消费布局和例子见 [voice-3.3.3.3.json](../fixtures/protocol/voice-3.3.3.3.json)。不能将阿富汗 CRC 当作其它地图的合法值。

`uk_m1_`、`uk_m2_`、`uk_f1_`、`uk_f2_`、`ab_m1_`、`ab_m2_`、`ab_f1_`、`ab_f2_` 各有 52 个名字，共 416 个角色声音名字。`uk_1mc_`／`ab_1mc_` 为指挥官播报；`rb_1mc_`、`za_` 等其它模式声音保留于完整清单。

以下例子来自阿富汗已初始化捕获 `client-1791342759140/net-constants-1791342878732.json`，NCS CRC 为 `0x1f3a0a82`，捕获 SHA-256 为 `d551f582aad7f759ce074a3a777d71cc71bf72cf3bb84a12ff95aae53164b20f`。其它地图按各自目录查找。

| 候选事件 | TF／UK alias 与 selector | SC／AB alias 与 selector |
| --- | --- | --- |
| 团队竞技模式介绍 | `uk_1mc_tm_deathmatch` / 1443 | `ab_1mc_tm_deathmatch` / 797 |
| 开战指令 | `uk_1mc_boost` / 1216 | `ab_1mc_boost` / 558 |
| 入场音乐 | `uk_spawn_music` / 1991 | `ab_spawn_music` / 1931 |
| 胜利播报 | `uk_1mc_mission_success` / 1384 | `ab_1mc_mission_success` / 737 |
| 失败播报 | `uk_1mc_mission_fail` / 1383 | `ab_1mc_mission_fail` / 735 |
| 平局 | `uk_1mc_draw` / 1251 | `ab_1mc_draw` / 595 |
| 领先／落后提醒 | `uk_1mc_winning_fight` / 1472；`uk_1mc_losing_fight` / 1378 | `ab_1mc_winning_fight` / 828；`ab_1mc_losing_fight` / 730 |
| 时间将尽 | `uk_1mc_timesup` / 1442 | `ab_1mc_timesup` / 796 |

AlphaBuild `_music_and_dialog.gsc` 认为旧客户端缺少模式播报；本版明确有 `tm_deathmatch`，不能使用旧脚本的 `tm_death` 名字或据其注释禁用本版模式介绍。当前开场先播放本阵营 `tm_deathmatch`，其可靠命令被确认后等待默认3000ms，再播放 `boost`；入场音乐使用独立音乐通道。SC实采模式语音占用约2642ms，开战指令在其结束后约488ms进入声音通道，两条各只有一个原生播放handle，未互相截断。`mode_intro_gap_msec`允许1000..10000ms；该间隔是已经验证的服务端排程，不声称是官方GSC时机。`ldr_tm_deathmatch`是另一条真实资源，当前不额外播放它。

| 角色事件 | suffix | UK 男声 1 的 selector |
| --- | --- | --- |
| 换弹 | `inform_reloading_generic` | 1638 |
| 手雷／闪光／烟雾／震撼 | `inform_attack_grenade`／`inform_attack_flashbang`／`inform_attack_smoke`／`inform_attack_stun` | 1630／1629／1631／1633 |
| C4／阔剑 | `inform_attack_thwc4`／`inform_plant_claymore` | 1634／1637 |
| 确认击杀 | `inform_killfirm_infantry` | 1636 |
| 受伤／死亡 | `mp_pain`／`mp_death` | 1653／1648 |
| 疾跑 | `mp_sprint` | 1663；由本地前缀初始化路径装配 |
| 快捷口令 | `mp_cmd_*`／`mp_rsp_*`／`mp_stm_*` | 目录保留攻击、跟随、回应、敌人位置等声音 |

`ui_mp_timer_countdown`、`weap_sniper_breathin/out/gasp`、`firstblood`／`headshot` 名字不在这套常量中。NCS 缺名字不证明资源不存在；倒计时和呼吸要另行确认 alias，才能动态注册或由本地初始化。爆头判定完成不代表爆头播报已实现。旧 battlechatter 的 `inform_casualty_generic` 也不在本版银行，目录不为缺失项编造编号。

## 角色前缀与触发位置

发给客户端的 prefix 使用大写：`UK_M1_`／`UK_F1_`／`AB_M2_` 等。`0x45567f` **直接比较 prefix[3] 与大写 `F`**；小写 `uk_f1_` 会进入男性呼吸资源分支。alias 清单中的小写名字与命令的大写前缀分别保留。

`0x4555c0` 在非 `zm` 模式注册 `<prefix>mp_sprint` 到 VA `0x01a1f7f4`，并注册普通或带 `_f` 的狙击吸气、呼气、喘气到 `0x01a1bdfc`／`0x01a1be00`／`0x01a1be04`。prefix 初始化这些自动声音，不会自动产生换弹、投掷和击杀喊话；本地疾跑／呼吸不应另排服务器声音。

本版实际 `mp/factiontable.csv` 的第7列确认taskforce_141→UK_、shadow_company→AB_；无需再以Alpha表为唯一依据。角色包到编号1／2的官方完整映射仍待恢复。配置 `voice_variant` 明确选择共享的1或2，默认1，不声称已恢复官方逐角色编号。`game_appearance` 已恢复包的 gender／原生 faction；接口让调用方明确提供 variant，不根据模型名字猜编号。Axis／Allies、房间阵营和角色原生 faction 的枚举不同，复用已有房间／外观转换。

| 接入位置 | 事件来源与边界 |
| --- | --- |
| 玩家出生与换角色 | 按已提交的角色资料发本地 prefix；Bot 喊话按各自资料选 alias，不能反复设置本地 prefix 模拟 Bot |
| `game_connection` 首个快照 ACK 与 HUD 规则就绪 | 已按连接／对局维护一次性状态；首个快照和HUD规则ACK后播放入场音乐与模式介绍，模式命令ACK后留出间隔再播开战指令；重复快照不重复介绍模式，结算取消尚未播出的开战指令 |
| `game_player` 实际进入换弹状态 | 消费权威事件环的RELOAD_START_NOTIFY；按每位角色序号识别开始，持键、重传和逐发补弹不重复喊同一次开始 |
| `game_grenade` 实际生成投掷物 | 投掷物实际launch成功后选语音，失败、准备阶段、死亡掉落和手中爆炸不喊投出；未知类型不猜其它语音 |
| `game_combat`／`game_match` 伤害、死亡、击杀提交 | 已接玩家实际受伤和已提交死亡；击杀确认延迟750ms，按出生／死亡代数绑定原身体，死亡、重生、回放或结算取消；Bot非致命受伤喊话尚未接入 |
| `game_match`／`advance_match_summary` | 最终回放结束后、前三名／结算展示前，按实际赢家播一次胜负或平局；领先变化和时间／分数提醒仅在候选目录中，尚未接触发 |

指挥官和音乐可对指定听众使用 `p`。角色喊话若保留位置，应走 EV3／源实体，并由服务器过滤听众。参考 `_battlechatter_mp.gsc` 使用同队、排除发声者、附近同时只一人说话、约 2 秒占用和 1000 单位范围；当前对换弹／投掷／击杀喊话采用该参考节流策略，尚未确认为本版官方规则。受伤音按角色和生命代数单独节流2000ms，受伤／死亡不占用队伍喊话的间隔；自伤后紧接闪光投掷已实采两种声音均播放。默认排除发声者自身，可用 `hear_self_chatter` 在独立验听准备中包含自己。回放期间仍推进角色事件游标，退出回放不补喊旧换弹；延迟喊话同时检查实际生命代数，漏掉死亡帧也不会落到新身体。资源采集模式不产生新语音。

## 工具与验证

[ClientVoice.cs](../tools/ClientVoice.cs) 通过当前 asset hash 的 type11 收集 alias 头、变体数量和未解码字段候选，读取相关 GSC／CSV 与自动声音句柄。仅使用已有 `VM_READ` 权限，无游戏函数调用或内存写入。

```sh
python3 tools/client_state.py --build --prefix-name wine10-loadout-build-20261007
python3 tools/client_state.py --experiment research-output/gamehub/实际实验/experiment.json --voice-assets
python3 tools/client_state.py --experiment research-output/gamehub/实际实验/experiment.json --net-constants
```

编译使用独立prefix。当前资源文件能正常读取，`ff_init_common_gfx_V43.ifs`的SHA-256为`a4bac3f12ef090773ee5fc5017e5f88dae600f2d81005e0f67257c02b78ff667`。原生验收只修改诊断账号副本的角色／投掷物和短局时限，普通账号资料未作诊断写入；各已结束诊断核对了证书恢复。

`--voice-playback`固定采样20秒，可配合`--sample-ready`等待首次采样再发送普通游戏输入。采集当前声音通道的alias／播放handle／原生MP3目录、声音事件的源实体／selector／一次性latch。实际type11 alias可具有flags0／2，不能据旧flags过滤将其判为缺失；目录将有效采集标为`captured_alias`，保留原flag字，不把头存在当作实际播放。

macOS音频通过ScreenCaptureKit仅捕获指定游戏进程，没有使用麦克风。完整采样、音频量级、短片段与结果在`research-output/voice-acceptance*`；主证据索引为`research-output/voice-acceptance/proof.json`。

| 实际实验 | 已核对内容 |
| --- | --- |
| `client-1791887222484`／安全屋 | UK入场音乐和开战指令；UK_M1换弹，Bot女声击杀喊话；用户确认开场／换弹声音正常 |
| `client-1791887684896`／安全屋 | 角色649使用AB_F1前缀，三种狙击呼吸资源均带_f；女声换弹与Bot死亡喊话；本版factiontable实读 |
| `client-1791888891468`／坠机之地 | AB模式介绍→开战指令；女声手雷与换弹；最终回放→前三名→胜局结算，胜利音乐与mission_success各一个handle |
| `client-1791889211040`／坠机之地 | 女声手雷、闪光、换弹和实际受伤；源实体0的selector882／881／890／905均出现原生声音事件；闪光投出不被受伤音节流；胜局回放／结算再次完整通过 |
| `client-1791889650864`／坠机快速入口 | UK_F1前缀和女性呼吸资源；TF女声手雷／换弹；TF胜利音乐与mission_success，AAR玩家及赢家均为team2 |

角色声音选到的真实文件目录包含`voiceovers/codo_mp/sc/f1`，不是仅根据alias名字推测性别。声音事件eType25的latch经历0→1，同一事件跨多个快照保留同一身份；模式／开战／胜利在各自阶段分别只有一个播放handle。当前声卡输出录音约114..117秒，能量非零且峰值小于1，没有量化削波；用户确认模式介绍与开战指令两段都完整播放，开场／换弹声音正常；具体音量偏好仍可按设置调整。

[prepare_voice.py](../tools/prepare_voice.py) 可使用已有的初始化采集生成目录：

```sh
python3 tools/prepare_voice.py \
  research-output/gamehub/client-1791342759140/net-constants-1791342878732.json \
  --game-state research-output/tdm-afghan-hud-state.json \
  --output research-output/voice-diagnostic/mp_afghan-voices.json \
  --state-output research-output/voice-diagnostic/mp_afghan-state.json
```

`--state-output` 必须为新文件，复制原准备并解析已有相对资源路径，添加绑定的 `voice_file`。`--voice-variant 2`可选另一套声音；`--mode-intro-gap-msec`指定模式介绍与开战指令的间隔；`--hear-self-chatter` 用于包含自身喊话的独立验听。可加 `--voice-assets` 合并同图、同 CRC、state10 的资源采集。缺少加载证明时标为 `constant_only`；`captured_alias`也只说明头与变体存在，不代表样本就绪或听感。准备器拒绝未初始化捕获、错误 CRC／布局／常量顺序、大小写歧义及跨图 alias 数据。

`sh tools/check.sh`的fmt、Clippy、Rust和110项Python检查通过。回归覆盖声音／源实体字段及到期删除、无重复重传、女性前缀、开局／结算只播一次、事件环回绕、同队过滤、附近节流、回放抑制，以及死亡／漏帧重生取消延迟语音。目录检查覆盖CRC、顺序、跨图拒绝、原准备保留和相对路径解析。

正常声音目录在`research-output/native-voice-resources/`；`enabled.json`记录23份地图准备的voice_file和原状态备份，普通分数／时限未改。`tools/play.py`沿用原地图catalog和账号即可加载已启用语音。

23份独立准备已经在release模式逐图实际加载并核对四种男女／阵营前缀，见`research-output/voice-acceptance/captured-preparations-release.log`。Rust回归另覆盖模式命令ACK后的延迟、结束时取消开战指令，以及受伤节流不压掉投掷喊话。验收工具选取Wine进程的最大主窗口，避免录音时的小浮动窗接收准备按键。

尚未恢复：官方逐角色voice variant映射、完整官方GSC排程、领先／时间提醒、爆头／首杀专用播报、Bot非致命受伤喊话。其它地图的资源绑定已复算与加载，不等于每个场景的空间衰减和每种语音都逐项验听。
