# 快照、输入与复制历史

本版快照、输入和 ACK 已支撑实际权威对局。共享多人每连接保留独立复制历史，模式覆盖与原生证据见[多人验收](MULTICLIENT_ACCEPTANCE.md)。

## 线格式

入口 VA `0x46d170`：opcode0、serverTime LE32、delta 距离 byte、flags byte。初始完整基线使用 flags8；客户端确认后切换普通帧，持续重发完整初始化状态不能替代正常渲染帧。

playerState 入口 `0x5a4230`，表 `0xc55f70` 为 235 项 `(offset,bits,hint)`，与 Alpha 的 244 项不同。字段间距和字段值分层编码；整数字段先处理 width%8 位再处理整字节。`-88` 为 LE32 原始位 XOR，`-87` 为 short 角度，不能混用。

PS 后有 14 个顶层数组／扩展段，包括 stats、clip、equipped、reserve、扩展结构、objectives、HUD 和索引列表；再依次发送动画命令、actor、entity、client、glass 与 EOF。entity 结束符为 2047，各段重置实体索引缓存。

本版字段事实保存在 `fixtures/protocol/player-state-fields-3.3.3.3.json`，表 SHA-256 为 `56b46d3430513982131602b42ce6a8f9c376320b2de8e383627676c88d9c2495`。提取入口为 `tools/extract_game_layout.py`。

## 输入解码

依据 writer `0x5a1be0`、包构造 `0x4602a0` 和默认 base `0x5a18e0`：每包 1～32 条 usercmd，从已确认 PS 建立初始 base，包内逐条 delta。三分支尾标记为 7／2／5，melee 带 11-bit 实体，移动尾部为预测 origin 三个 LE32、预测时间及额外 word，最后 EOF3。

key 来自 message ACK、checksumFeed 和最多 32 字节服务端命令 hash；与 native XOR 的 challenge 不同。完整包校验后才提交序号、ACK、可靠命令和输入；截断、非法计数／分支或缺 EOF 不推进连接。预测坐标只用于对照，不写入权威世界。

输入 base 随当前武器、offhand、alt、视角、ADS 和玩家标志改变。服务端按 ACK 选择对应已发快照，不能使用尚未确认的新 PS 或固定零值。时钟与装备约束见[玩家出生](PLAYER_BOOTSTRAP_PROGRESS.md)。

## 历史、预算与压缩

网络基线、输入 base 与演出回放分别保存。只有实际发出的快照登记发送历史，可靠消息不替代世界基线。共享世界按接收者置换本地 slot0，原始参与者身份和权威时钟不变。

大 gamestate 和密集快照共用原生 zlib／XOR 路径，再按需要分片。可靠窗口不足时使用独立消息推进 ACK，保留世界事件、尸体及回放状态；不可通过截断世界或伪造已发送序号绕过预算。[分片支持边界](NATIVE_CHANNEL_PROGRESS.md)不等同于完整丢包恢复。

回归包含 12 名活人、8 具尸体、32 条击杀及持枪实体的压缩／还原，字段表和多个位池起点的独立解码。原生密集战斗已完成个人／最终回放、前三名与结算；更多武器和事件组合仍需分别验收。

`--snapshot-probe` 是显式有界的诊断流，默认 60 帧、最大 18000 帧；正常游戏使用持续模式，见[本地游玩](LOCAL_PLAY.md)。
