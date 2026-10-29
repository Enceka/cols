# 剧情模式与服务端接入

目标为 CODOL 3.3.3.3，客户端 SHA-256 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。剧情可以沿用现有 Rust 后端的认证、分配、UDP、移动和战斗基础继续接入；当前尚不能开局游玩。最小目标应为首关“身份不明”对应的 `coop_training_mp_cargoship`，原生模式为 `coop`，地图表限定一个玩家。

本版数据含 11 条有章节编号的剧情地图，当前版本列开放 10 条；这表示前端版本准入，不表示资源完整、账号已解锁或玩法可用。显式首关采集入口已实现无 `21/1` 的随机密钥分配与原生开局校验，默认 TDM 分配不变。首关原生分配/握手与资源初始化已通过，真实采集见[剧情资源采集](STORY_RESOURCE_CAPTURE.md)。当前缺口为可游玩的`coop`世界、任务运行、剧情角色和通关持久化。当前 Rust 无 GSC VM，也没有剧情任务模块；不能靠修改地图名完成剧情接入。

## 关卡、前端与资源

只读前端捕获实测 `build_version_column=2`。`MatchService.isRowValid` 只读取所选版本列；前两列的值不决定本版是否开放。`QuickMatchType` 将剧情归为 `MapType.COOP / CoopMapType.STORY`、Hero Ops，模式字符串为 `coop`；生存、撤离和机械僵尸有独立分类。

`mp/coMapList.csv` 的零基列 3/4 为原生编号/地图名，10/11 为最小/最大玩家数，12/13 为章节/关卡，22 为前置解锁标记，33 为排序。所有下表剧情行均为 1～1 玩家。货轮的原始章节/关卡值为 1/2，排序 101；应保留这些实际值，不为了首关显示改成 1/1。

| 原生编号 | 地图 | 章节/关卡 | 本版版本列 |
| --- | --- | --- | --- |
| 17 | `coop_training_mp_cargoship` | 1/2 | 开放 |
| 19 | `coop_take_mp_fav_tropical` | 1/3 | 开放 |
| 20 | `coop_take_mp_estate_tropical` | 1/4 | 开放 |
| 21 | `coop_defense_mp_crash_tropical` | 1/5 | 开放 |
| 22 | `coop_evasion_mp_con_spring` | 2/1 | 开放 |
| 23 | `coop_sniperfi_mp_con_spring` | 2/2 | 开放 |
| 24 | `coop_escape_mp_storm_spring` | 2/3 | 开放 |
| 25 | `coop_ac_mp_hunted_spring` | 2/4 | 开放 |
| 26 | `coop_suspension_mp_bridge` | 3/1 | 开放 |
| 27 | `coop_chopper_mp_suburbs` | 3/2 | 开放 |
| 28 | `coop_defense_mp_suburbs` | 3/3 | 关闭 |

前端 `TutorialMap.ONBOARD_CARGOSHIP / CB3_ON_BOARD` 均指向货轮首关。地图表还包含剧情简介、不同难度的目标/提示、完成标记与下一关关系。前端展示这些文本不会执行服务器任务。

362 个 IFS 的既有只读清单中，上述 10 条开放剧情图均观察到本图 `.ff`、`_load.ff`、`_const.ff` 路径。货轮对应的主包为 `ff_init_coop_training_mp_cargoship_V29.ifs`，常量包为 `lf_init_coop_training_mp_cargoship_const_V30.ifs`，另有纹理/声音包。EXE 也含 `common_gametype_coop` 与本图 fastfile 名称。

其余 9 条开放剧情图各有独立 `main/video/<map>_load.bk2 / _outro.bk2` 文件。货轮没有同名外置影片，不能因此判断首关资源损坏或虚构该文件依赖。关闭的 `coop_defense_mp_suburbs` 未在清单观察到上述三个 fastfile 路径。

IFS 清单中没有独立 `.gsc/.csc` 路径。这只说明清单没有列出散装脚本；脚本仍可能位于 fastfile 内部。首关已有脚本／FX资源捕获，但本版完整剧情GSC、触发图和依赖闭包尚未验证，也没有验证容器块完整性与补丁合并结果。不能把 fastfile 名称、影片文件存在或同底图的 TDM 碰撞当作完整剧情资源证据。

## 开局分配与密钥

真实首关既有采集  的 原生画面证据 显示选中“身份不明”并停在“战斗开始”。packet 039/041/042 依次为 `11/13` 建 channel、`21/5` 查询游戏服、`11/4` 房间消息；所有已记录的业务帧均未出现 `21/1`。查询的实际参数为 `query=1, offset=0, maximum=10, build=-688906341`，没有地图字段。这一条采集不证明全部剧情关卡流程一致。

默认 `backend/rooms/mod.rs` 的 `21/5` 分支要求 `channel.game` 存在，密钥来自客户端 `21/1` 描述；缺少描述就返回 `game_allocation_requires_live_key`。显式 `--story-capture-map coop_training_mp_cargoship` 允许拥有者的单人 channel 不经 `21/1` 分配随机 security ID/key；分配与登录身份/channel/build 绑定，重试复用，退出、换登录和过期撤销。它没有改变默认 TDM 前置条件。

本版客户端的静态消费者支持进一步恢复这个路径：

- `0x701db0` 的分配结果解码器读取 16 字节 Blob 至结果结构 `+0x1cc`；基类 `0xa55700` 读取 8 字节 security ID 至 `+0x10`。
- `0x6d4530` 从结果复制 ID、37 字节地址和上述 key 到 61 字节服务器记录，key 位于记录 `+0x2d`。它提供分配回包密钥会被保留的直接静态证据。
- 游戏服查询调用链为 `0x6e3310 → 0x6d4900 → 0xa56f50`；返回经 `0x6d4790 → 0x6eb2a0 → 0x6eb140` 将完整 `0x228` 字节结果复制到房间查询缓存，包含上述 key。

**据此推断并实现诊断入口**，无 `21/1` 的分配由 Rust 游戏服生成随机 key，并在同一条分配记录返回 security ID/key/真实地址，让客户端采用该记录。诊断结果中的未知属性使用零值，首关已实测接受该分配记录并完成booking、随机P-224和认证DATA；该证据尚未扩展到其它关卡。现有训练路径继续保留客户端上传 key 的行为。

恢复时应把分配身份绑定到当前 title/UID/登录 seed/channel/build，重试复用分配，到期、退出和换登录撤销；使用实际绑定的 UDP 接收端。查询只有 build，不能在此时凭 query 值确定所选关卡。地图/难度应从后续原生房间消息及 `mmstart` 确认，再核对已准备的剧情目录，缺失时明确拒绝。

原生 `0x6e6bf0` 同时生成 playlist 与显式地图开局命令；静态格式分别位于 `0xc7e740`、`0xc7e668`、`0xc7e6d0`。显式形式包含人数、`g_difficulty` 或 `2810`、地图及其它设置。首关究竟使用哪一形式仍需真实回包后采集，不应预填训练的 playlist 命令。

## 当前 Rust 接入边界

| 能力 | 当前可复用部分 | 剧情缺口 |
| --- | --- | --- |
| 账号与游戏通道 | 认证、大厅、channel、booking、P-224/加密 DATA、连接与 stats 上传；显式首关服务端密钥分配 | 分配结果的客户端接受性及真实剧情握手 |
| 地图与资源 | NCS/CRC 计算、实体与碰撞提取、gamestate、资源目录 | 本图初始化证据、出生、剧情实体和 `coop` 常量；不能复用 TDM CRC |
| 玩家与战斗 | 背包/武器、移动、命中/伤害、战斗事件 | 剧情的装备限制、受控移动、死亡/重试规则 |
| AI | 已实现 TDM Bot 的移动、导航、交战与动画 | 跟随、守点、指令、剧情 NPC、脚本动画及载具；当前 Bot 使用 PLAYER 席位 |
| 任务与表现 | 已有声音、动态物件及部分 HUD/事件基础 | 触发器、目标、字幕、镜头、演出、成功/失败和检查点 |
| 进度 | 档案读取/持久化与本版 schema 研究工具 | 服务端权威通关结果、解锁写入及重登验收 |

目录及可玩世界按地图／模式校验，正常目录接受已准备的 `war`／`cha`；剧情资源初始化使用独立 `story_resource_state`，不向TDM目录写入剧情世界。`TdmMatch`按分数／时限运行，不能作为`coop`任务胜负规则。

建议为 Rust 玩法增加独立的剧情状态，使用经过本版证据确认的触发事件与资源标识；首关可先实现所需的有限任务状态机。若要运行完整原始 GSC，则还需本版脚本格式、VM、原生方法、线程/notify 与角色系统的独立实现。没有完整脚本数据时，不能把猜测出来的任务流程标为原剧情恢复。

## 通关与解锁

`MatchService` 用地图第 22 列查询前置解锁，以 `cf_ch_<map>` 判断通关。货轮没有前置标记；第二关要求 `cf_ch_coop_training_mp_cargoship`，后续剧情逐关引用前一关完成标记。

本版 `mp/unlockFlags.csv` 将货轮完成标记映射到编号150，其它现存剧情标记在151～160，并保留161/162的额外剧情标记；编号顺序不等于章节顺序。实际准入读取`mp/playersecuredata.def v19`的`unlockFlagsNew` Int16字段，最低值1；旧`unlockFlags`布尔字段不能单独恢复准入。独立测试账号已按这13个标记解锁，本版10个可见关卡的菜单已验证；实际胜局提交/持久化协议仍未验证。

当前私有档案RPC不支持任意写入，游戏 `stats` 上传也明确不是权威档案更新。剧情成功结果需要服务端确认、幂等保存，按本版 schema/CRC/投影构造私有档案，并核对下一关解锁、失败不解锁、重试和重登行为。不能用全解锁测试账号的界面作为通关持久化证据。


## 首关任务运行时

剧情任务定义是独立文件，按 `schema_version: 1`、客户端哈希、`map`、`mode` 与完成标记校验；`stages` 每项带 `id`、可选 `checkpoint` 和四类目标之一：`reach`（坐标与半径）、`eliminate`（数量）、`survive`（秒数）、`finish`（坐标与半径）。首关只接受 `coop_training_mp_cargoship` 与完成标记 `150 / cf_ch_coop_training_mp_cargoship`，且 `finish` 必须是最后一段。

`StoryMatch` 是任务权威：按玩家实际位置、存活状态和已记录击杀推进目标，死亡时回到最近检查点并累计尝试次数，通过 `StageStarted / Checkpoint / Retried / Completed` 事件通知连接；`Outcome::Completed` 只在 `finish` 完成后出现。连接侧在无 TDM 世界的分支推进任务，重试时按检查点原点重新出生并重置生命、装备与出生时间，完成时记录完成标记名称。

剧情世界准备在普通准备文件上增加 `story_mission`：加载时要求 `map`/`mode` 与任务一致、没有 TDM 配置、并且存在真实碰撞与武器；开始点取准备文件里已经贴地的 `spawn_origin`，并覆盖任务文件里的占位起点。服务端用 `--story-world <准备文件> --story-mission <任务文件>` 发布该世界，`story-capture.json` 会记录 `world_simulation` 和 `mission`（地图、段数、完成标记）。

`fixtures/protocol/story-first-mission-synthetic.json` 是格式与运行时的合成样例，不是本版首关真实目标数据。

## 首关玩法的剩余验收

1. 以已通过的独立分配、握手和初始化为基础，校验首关资源依赖闭包；正常账号和TDM资源保持隔离。
2. 只绑定 `coop_training_mp_cargoship / coop`，复用已采资源顺序、CRC、实体和碰撞，恢复真实开始点并验收出生。取得失败时保留具体阻塞，不回退阿富汗等 TDM 图。
3. 任务定义、目标推进、检查点重试与完成事件已实现，但目标数值仍是合成样例；仍需取得本版首关脚本/触发数据与角色资源，替换为真实目标并接入敌人生命，才能验收任务开始、目标触发、角色行动、死亡/失败、重试和成功。载具、镜头与脚本动画按实际依赖决定实现次序。
4. 验收首关成功后第二关解锁、失败不解锁、重复结算不重复发奖及重登保留进度，再扩展后续剧情。多人 PVE 应独立评估；本版剧情表的玩家上限为 1。

## 证据与复核

原始捕获、反编译输出和资源表留在忽略目录，不发布剧情脚本、文本、影片、账号或密钥。首关采集已通过原生分配/握手、stats及gamestate资源初始化，CRC复算一致；没有出生或通关证据。完整结果见[剧情资源采集](STORY_RESOURCE_CAPTURE.md)。

本地汇总为 `research-output/story-study/evidence.json`，记录来源哈希、版本列、最小关卡元数据、fastfile 清单观察、外置影片存在性、完成标记的 schema 位置及旧抓包查询。可在 `server/` 运行 `python3 research-output/story-study/analyze.py` 重算；脚本只读取本地证据，不调用客户端。

### 准备与采集入口

`tools/prepare_story.py` 从只读前端、私有档案定义及 IFS 清单生成 `catalog.json` 和 `first-map.json`。它按版本列2保留开放/关闭状态、原始章节顺序、单人容量和完成标记的 schema 位置；不会生成可游玩的世界或写入 TDM 目录。本机元数据目录为 `research-output/story-integration-preparation/`；首关资源初始化和独立CRC已验证，完整依赖闭包与剧情玩法仍未验证。

```sh
python3 tools/prepare_story.py \
  --frontend ASSETS_JSON \
  --assets ASSETS_JSON \
  --resource-lists research-output/ifs-lists-3.3.3.3.json \
  --output research-output/story-preparation-new

sh tools/cargo.sh build --locked -p codol-server
python3 tools/gamehub_run.py client --prefix-name wine10-story-capture --seconds 900 \
  --title-id-hex 3ea --rust-auth --local-auth-tickets --rust-lobby --rust-stun \
  --rust-game --local-services --trust-lab-cert --quiet-network \
  --service-data-dir research-output/story-integration-preparation/service-data \
  --story-capture-map coop_training_mp_cargoship
```

采集使用预先复制好的独立服务数据、独立 Wine prefix，并遵守现有客户端锁和监听端口排他规则。`serve-auth` 的同名选项要求游戏 UDP，并拒绝与 `game-state`、地图目录、Bot 房间或持续玩法组合；harness 还要求显式 prefix/服务目录并拒绝正常 `client-work/server-data`。正常服务不应为准备工作重启。

该入口只接首关的显式 `map / g_gametype coop` 原生开局形式（允许实测的末尾空`2654`参数），校验单人容量、重复设置和不支持的命令，保留难度元数据；不执行收到的设置。传输目录的 `story-capture.json` 明确记录没有世界/玩法快照。不接受其它地图、TDM模式或训练 playlist 替代首关。原生实际命令若不在已核对形式内，先采集并核对消费者再扩展。

协议、分配、退出撤销、身份隔离、密钥重试/轮换、开局绑定、CLI互斥与元数据准备均有自动验证。独立工作树完整fmt/Clippy、534项Rust和136项Python检查通过；合入后主工作区剧情相关测试通过。随机空闲端口的实际后端启动与正常退出已验证，记录为`research-output/story-integration-preparation/backend-ready.json`，准备好的可执行文件为同目录`codol-server`。准备文件和后端启动不代表剧情任务执行结果；首关新原生分配/DTLS、本图资源初始化与碰撞/武器/角色捕获已完成，见[剧情资源采集](STORY_RESOURCE_CAPTURE.md)；可游玩的剧情世界仍待实现。

主要依据：

- 只读资源采集：只读前端、79×37 地图表、当前版本列。
- `research-output/remaining-map-analysis/frontend-source/scripts/codo/` 下 `constants/table/Map.as`、`constants/QuickMatchType.as`、`constants/TutorialMap.as`、`services/MatchService.as`：表字段、剧情分类、首关绑定、准入与解锁消费者。
- 只读资源采集：本版 unlockFlags 表与私有档案 schema；不读取账号通关值。
- `research-output/ifs-lists-3.3.3.3.json`：IFS 路径观察，范围限制见 [资源准备](MAP_RESOURCE_PREPARATION.md)。
- `000001`：已有首关业务采集，进一步背景见 [房间](ROOM_PROGRESS.md)。
- `research-output/ghidra/story-security-flow.c`、`story-query-callers.c`、`story-selected-session.c`、`story-security-endpoint.c`、`matchmaking-record.c`、`matchmaking-base.c`：固定客户端静态消费者，地址均为首选映像 VA；不是运行时实机完成证明。
