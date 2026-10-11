# 3.3.3.3 快照与输入

2026-10-07，接续 [native通道](NATIVE_CHANNEL_PROGRESS.md)。**最新已实际进入阿富汗地图，显示场景与HUD**，连接状态10，客户端持续返回native ACK和输入。武器、权威移动、完整玩家实体和AI尚未实现；不是可玩的训练对局。文末记录普通帧切换与进图验收，前面的黑屏实验属于前序诊断。

## 目标格式

- 快照入口VA `0x46d170`：opcode0、serverTime LE32、delta距离byte、flags byte。无delta的初始帧必须带flags8，成功后保存到 `0x07996128`；playerState基线选择占1bit。
- 玩家状态入口 `0x5a4230`，字段表 `0xc55f70` 为235项，每项12字节(offset, bits, hint)，不是Alpha的244项。读取ROV bit、8bit lc；字段间距=1用1bit，否则0bit后以4bit、15续接方式编码delta-1。
- 当前只发送index0的commandTime（codec -97，`0x5a20c0` 的绝对LE32分支）和stats[0]=100。其余字段来自空基线。
- 玩家状态后面有**14个**顶层数组/扩展段：stats、clip、equipped、reserve、四个扩展结构段、objectives、HUD、四个索引列表。先前只看到尾部新增三段的分析不完整；必须检查整个读取函数。
- 后续为空动画命令门0（`0x5a3b50`）、actor门0（`0x46cc60`）、entity绝对索引2047结束符（先0/1两bit再11bit索引，`0x46c2f0`）、client门0（`0x46c840`）、glass门0（`0x43e070`）、外层EOF6。
- 整数字段codec `0x5a2510/0x5a2470` 先读width%8位，再读整字节，不能与普通连续位池混淆。`-88`是LE32原始位XOR，`-87`是整字节short角度。字段值编码与字段索引间距为不同层。

## 真实快照与输入证据

`client-1791344097175` 开启最初的60帧诊断：`client-state-1791344142794.json` 读到状态10、valid1、flags8、serverTime/commandTime1900、snapshot消息22、health100；之后消息63、时间3950。客户端返回 ACK4，随后 ACK63。用户观察到可打开记分板，之后背包菜单显示，最终“服务器连接错误”；这不是可玩验收。

这轮诊断3秒后主动停止发送，而且当时移动opcode尚未支持。随后恢复输入格式并对该实验6个native包离线重放，全数成功，覆盖1条及32条usercmd、可靠菜单命令前缀和所有尾字段。

`client-1791344898487` 改为6000帧诊断流：超过5000帧时305个入站事件无解析错误。`client-state-1791345086196.json` 状态10、valid1、snapshot消息1781、时间89750、clientCommandAck18；入站ACK也持续推进。这证明快照、输入及可靠命令接收能持续互相确认；**命令未被执行为游戏规则或物理运动**。

输入依据是目标writer `0x5a1be0`、包构造 `0x4602a0`、默认base `0x5a18e0`，没有直接照搬Alpha：

- opcode0/1之后byte计数1～32，每包从当前PS构造初始命令base，包内逐条delta。
- 三种分支尾标记7/2/5；分支位、fire位及字段值使用不同阶段key。
- 相较Alpha，melee分支多一个11bit实体号；移动尾部为五个LE32（预测origin三个、预测time一个、额外word一个），最后才是3bit EOF3。
- key来自message ACK、checksumFeed和最多32字节服务端命令hash（`0x62d850`）。当前服务端命令为空、feed0；不要误用native XOR的challenge。
- 当前输入base仅适用于已下发的零角度、零武器和零buttons PS。之后修改这些字段时必须同步恢复对应base，不能继续默认零。
- 完整包解码后才提交native序号、ACK、可靠命令及最新输入；截断、错误分支tag、非法计数、缺少EOF不推进连接。保存原始usercmd但不执行控制台文本，不用客户端预测origin作为权威位置。

## 运行与边界

```sh
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --prefix-name wine10-recovered-20261007 --seconds 900 --title-id-hex 3ea --rust-auth --local-auth-tickets --rust-lobby --rust-stun --rust-game --game-state research-output/training-game-state.json --snapshot-probe 6000 --local-services --trust-lab-cert --quiet-network
```

`--snapshot-probe` 不带数字默认60帧，允许1～18000帧，需显式gamestate配置；按20Hz调度，达到数量或admission失效后停止。它是诊断流，不是完成的比赛循环；达到预算后仍可能超时断开。入站抓包预算随帧数有界增加，避免原2048包上限在较长测试中突然丢弃输入。snapshot单独保存发送序号/时间、DTLS正文及密文；所有原始数据保留在忽略目录。

新增只读字段包含完整基线的valid、flags、serverTime、message、commandTime和health。`--map-entities` 读取type19资源的name/entityString/length（header前三个word），不调用游戏代码。实测 `client-1791344898487/map-entities-1791345085525.json` 导出 `maps/mp/mp_afghan.d3dbsp` 的241037字节实体文本；第一条团队出生点 `mp_tdm_spawn_allies_start` 为 `-472 -800 -113.4`，角度 `0 30 0`，另有全局intermission位置。

## 下一步

先从地图资源准备有来源的出生点，恢复视角高度与实体未绑定标记。目标 `0x4e95e0` 将PS+0x1f0写为2047，`0x5227bd` 比较它决定链接视角分支；`0x4e8e11/0x4ebc1b` 将PS+0x1b4写为2047。不要保留零值而把实体0误作有效链接。

之后补充local client/entity记录、真实装备和server规则。当前没有地图碰撞、位移积分、命中、AI，也没有完整可靠重发和snapshot delta历史。应以实际场景、HUD、操作和持续同步验收，不能以状态10或菜单可见代替。

## 真实场景与HUD验收

`client-1791345608656` 首先验证地图来源的出生点与视角字段：`client-state-1791345711612.json` 中origin为 `[-472,-800,-113.4000015]`、viewHeight=60、viewLockedEntity=viewLinkedEntity=2047，快照有效、ACK推进，但画面仍黑。总4104帧、247个入站事件，无服务端解析错误。因此不能把黑屏只归因于原点坐标。

`client-1791345984530` 在首个基线获得native ACK后，将后续帧flags从8改为0。保留delta距离0，令actor/entity/client历史引用已缓存的完整基线；playerState选择位仍为0，独立从零基线完整写入。这样不需要假设某个尚未确认的delta序号已存在。

结果：`ui-1791346072400.png` 已显示阿富汗地形、植被、天空、雷达框和计分HUD。`client-state-1791346072294.json` 对照：缓存基线消息27、flags8、time2150；当前快照消息587、flags0、time30150，valid1、状态10。此时的黑屏解除发生在普通帧切换后；不再将“反复收到完整基线”视为渲染帧验收。

最后一轮6798帧、251个入站事件，无服务端解析错误，收到的最高native ACK=6627。先前6000帧输入实验最终465个入站事件、最高ACK6005，也没有解析错误。当前输入发送仍较稀疏（可见约33秒一包、每包最多32条）；后续需要检查客户端发包节流/前台状态，不能把解析通过当成正常实时操控。各实验已停止、临时CA均恢复。

地图准备与启动：

```sh
python3 tools/prepare_game_state.py research-output/gamehub/client-1791341394600/net-constants-1791341887328.json --map-entities research-output/gamehub/client-1791344898487/map-entities-1791345085525.json --output research-output/training-spawn-state.json
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --prefix-name wine10-recovered-20261007 --seconds 900 --title-id-hex 3ea --rust-auth --local-auth-tickets --rust-lobby --rust-stun --rust-game --game-state research-output/training-spawn-state.json --snapshot-probe 18000 --local-services --trust-lab-cert --quiet-network
```

`--map-entities` 必须对应目标版本、目标map名，选择第一条 `mp_tdm_spawn_allies_start` 的三维有限坐标，并记录原始采集SHA-256。不提交实体文本。准备文件省略该选项时仍支持原始零坐标诊断，新增字段保持向后兼容。

新增下发字段（target table index / PS offset / codec）：origin X=8/0x1c/-88、Z=10/0x24/-88、Y=11/0x20/-88；viewHeightCurrent=34/0x15c/-88；viewHeightTarget=38/0x158/-8；groundEntity=47/0xac/11；viewLockedEntity=61/0x1b4/16；viewLinkedEntity=68/0x1f0/11。整数编码为nonzero bit + width%8低位 + 剩余整字节。lc=69，范围内无hint2字段。视角角度、武器与otherFlags仍为零，输入默认base尚不支持任意玩家状态。

235项静态字段事实保存在 `fixtures/protocol/player-state-fields-3.3.3.3.json`，表SHA-256为 `56b46d3430513982131602b42ce6a8f9c376320b2de8e383627676c88d9c2495`，可用 `.venv/bin/python tools/extract_game_layout.py --output 目标路径` 从核对过EXE重建。测试对照这些独立提取值及真实客户端接受的48字节快照向量，防止索引、字段编码和保留位池退化。

快照调度使用tokio interval给出的计划时间，而非处理时刻，避免微小调度抖动把下一帧判成“尚未到期”而跳帧。仍有显式帧数、admission期限及实验运行时间限制；到预算末尾停止发送或进程关闭是实验边界，不能承诺永不断线。

最终检查：fmt、Clippy、87项Rust和17项Python测试全部通过；服务端调试可执行文件已重建。覆盖实际接受快照向量、14段结构、PS字段源表、出生数据校验、输入截断/tag/新增melee字段、发送节奏与预算、ACK后基线切换。只读探针已重新编译并在目标进程中验证。


后续进展见 [玩家出生与装备](PLAYER_BOOTSTRAP_PROGRESS.md)：时钟已修复、组合M4和30/90弹药已下发、输入基线随ACK选择；视角与模型姿态仍未通过实际操作验收。当前运行prefix为wine10-play-20261007。

## 2026-10-11 密集对局与回放退出的大快照

近战接触测试`client-1791687401144`将多名Bot放在较近的出生点，连续死亡后，个人回放退出触发`player snapshot exceeds native message budget`并使服务退出。该轮失败完整保留，不计作验收。不是客户端崩溃，也没有证据将它归因于近战伤害。

此前只对初始gamestate使用原生压缩，局内快照仍在发送前按未压缩长度直接拒绝。现局内也复用`compact_server_packet`：小消息原样发送，超限才按本版ACK高位标志压缩正文、再XOR。可靠命令仍有界发送；快照占满原预算时用独立可靠消息推进ACK，不截断事件/尸体、不伪造已发送序号。完整世界历史和输入基线仍只在实际发送快照后登记。

新增回归构造12个活人、8具尸体、32条击杀记录和持枪实体：原未压缩发包拒绝，压缩后按原生XOR→inflate完整还原，正文逐字节相等。原压缩预算、过大/不可压缩数据拒绝、小包字节不变检查继续保留；没有宣称已实现原生分片。

修复后的`client-1791687868088`连续两局完成回放、前三名及败局结算。实采7次压缩快照，原文1150..1209字节，线上包最大930字节；服务未再因该上限退出。证据汇总在`melee-combat-live-proof.json`，含原失败实验、原始日志、4次近战击杀及完整结算记录。该范围不等于所有武器/事件组合均不会超限。

`melee-delivery-check.log`完整347 Rust/50 Python及fmt/Clippy通过；本轮无截图。正常配置仍为7500分/10分钟，密集测试坐标仅保存在独立准备文件。
