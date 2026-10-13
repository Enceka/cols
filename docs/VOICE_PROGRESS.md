# 原生开局播报与角色语音

目标为未修改的 3.3.3.3 客户端与纯 Rust 服务端，EXE SHA-256 为 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。当前具备声音选择器、播放／停止命令、角色前缀、只读探针和目录准备器；新增开局、胜负和角色喊话尚未接入对局触发。资源身份、消费路径和听感验收分别记录。

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

[game_sound.rs](../crates/protocol/src/game_sound.rs) 提供 `SoundSelector::constant`／`dynamic`、`play_command`／`stop_command`／`spatial_event` 和 `VoicePrefix`。可靠窗口负责重发；业务事件只入队一次，不能每个快照重新产生播报，也不能同时发送同一声音的空间事件和本地命令。

## 资源目录与候选事件

真实已初始化 NCS 捕获按本版排序、去重和 CRC 算法复算，再与各图准备 CRC 核对。目录索引为 `research-output/voice-study/map-catalog.json`：22 张多人地图各有 1494 个原生声音常量、8 个动态常量；竞技场 `za_coliseum` 诊断准备有 1507／24 个。竞技场资源可分析不代表其 PVE 玩法完成。

完整名字、selector、候选事件和来源 SHA-256 在 `research-output/voice-study/maps/<地图>-voices.json`。由用户资源生成的完整目录保留在忽略目录；可提交的消费布局和例子见 [voice-3.3.3.3.json](../fixtures/protocol/voice-3.3.3.3.json)。不能将阿富汗 CRC 当作其它地图的合法值。

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

AlphaBuild `_music_and_dialog.gsc` 认为旧客户端缺少模式播报；本版明确有 `tm_deathmatch`，不能使用旧脚本的 `tm_death` 名字或据其注释禁用本版模式介绍。表中对应关系是候选；官方开场完整顺序、准确时机、是否同时使用 `ldr_tm_deathmatch` 等仍需本版脚本／实机核对。

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

TF→UK、SC→AB 的语义映射来自参考 factiontable／`_teams.gsc`，本版两套资源银行已核对；本版 factiontable 和角色包到编号 1／2 的完整映射仍需提取。`game_appearance` 已恢复包的 gender／原生 faction；接口让调用方明确提供 variant，不根据模型名字猜编号。Axis／Allies、房间阵营和角色原生 faction 的枚举不同，复用已有房间／外观转换。

| 接入位置 | 事件来源与边界 |
| --- | --- |
| 玩家出生与换角色 | 按已提交的角色资料发本地 prefix；Bot 喊话按各自资料选 alias，不能反复设置本地 prefix 模拟 Bot |
| `game_connection` 首个快照 ACK 与 HUD 规则就绪 | 按对局代数维护开局的一次性状态；完整顺序和音频加载门槛待实测，发送 gamestate 本身不证明就绪 |
| `game_player` 实际进入换弹状态 | 使用服务器接受的换弹开始；持键、重传和逐发补弹不能重复喊同一次开始 |
| `game_grenade` 实际生成投掷物 | 按接受的 grenade 类型选语音，失败投掷或仅进入准备阶段不播放 |
| `game_combat`／`game_match` 伤害、死亡、击杀提交 | 受伤喊话节流；延迟击杀喊话随死亡／离房取消，使用实际角色声音和听众 |
| `game_match`／`advance_match_summary` | 领先变化、时间／分数阈值、最终胜负各维护一次性触发；个人回放不是新开局，最终回放／前三名／结算时机分别核对 |

指挥官和音乐可对指定听众使用 `p`。角色喊话若保留位置，应走 EV3／源实体，并由服务器过滤听众。参考 `_battlechatter_mp.gsc` 使用同队、排除发声者、附近同时只一人说话、约 2 秒占用和 1000 单位范围；这些是参考脚本策略，尚未确认为本版官方规则。死亡或换角色后清理待播放喊话。

## 工具与验证

[ClientVoice.cs](../tools/ClientVoice.cs) 通过当前 asset hash 的 type11 收集 alias 头、变体数量和未解码字段候选，读取相关 GSC／CSV 与自动声音句柄。仅使用已有 `VM_READ` 权限，无游戏函数调用或内存写入。

```sh
python3 tools/client_state.py --build --prefix-name wine10-loadout-build-20261007
python3 tools/client_state.py --experiment research-output/gamehub/实际实验/experiment.json --voice-assets
python3 tools/client_state.py --experiment research-output/gamehub/实际实验/experiment.json --net-constants
```

编译使用独立 prefix。探针已通过 C# 编译；当前 `wine10-throwables-20261011` 出现 Wine server 无法连接，另一个项目 prefix 的游戏／只读探针超时，未得到有效的新 alias 采样。诊断使用账号副本且已核对证书恢复，正常账号与配置未因音频研究修改。记录在 `research-output/voice-study/result.json` 和 experiment 的 `wine.log`；环境故障未计为声音播放成功或失败。

[prepare_voice.py](../tools/prepare_voice.py) 可使用已有的初始化采集生成目录：

```sh
python3 tools/prepare_voice.py \
  research-output/gamehub/client-1791342759140/net-constants-1791342878732.json \
  --game-state research-output/tdm-afghan-hud-state.json \
  --output research-output/voice-study/mp_afghan-voices.json
```

可加 `--voice-assets` 合并同图、同 CRC、state10 的资源采集。缺少加载证明时标为 `constant_only`；`loaded_alias` 也只说明头与变体存在，不代表样本就绪或听感。准备器拒绝未初始化捕获、错误 CRC／布局／常量顺序、大小写歧义及跨图 alias 数据。

`sh tools/check.sh` 的 fmt、Clippy、Rust 和 102 项 Python 检查通过。新增回归覆盖动态／常量边界、零起始索引、16000 上限、14bit 事件、女性大写前缀、源排序／去重、CRC／loaded 顺序和跨图拒绝；23 份目录独立复算通过。

待验收：客户端运行环境恢复后，采集 alias／样本就绪和角色 voice variant 映射，再接入并隔离验听开局、男女换弹／投掷与胜负播报，核对声音、顺序、音量、空间衰减、死亡取消和重复抑制。当前对局未启用新增播报，编译、NCS 存在和可靠 ACK 不计为听音通过。
