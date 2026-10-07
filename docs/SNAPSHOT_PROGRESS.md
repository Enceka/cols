# 3.3.3.3 快照与输入

2026-10-07，接续 [native通道](NATIVE_CHANNEL_PROGRESS.md)。已实测进入连接状态10，客户端保存有效完整快照，并开始返回 native ACK 和输入。**尚未恢复正常场景/HUD、装备出生或物理模拟**，当前仍有黑屏及菜单显示。

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
