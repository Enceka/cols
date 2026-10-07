# 从已连接到实际对局

更新：2026-10-07。登录、训练分配、DTLS、challenge/connect、stats、心跳、可靠消息及 gamestate 已获 3.3.3.3 客户端接受。现在连接进入状态9，停在“等待玩家数据”。本文件规划后续世界状态；最新证据见 [NATIVE_CHANNEL_PROGRESS.md](NATIVE_CHANNEL_PROGRESS.md)。

## 1. 游戏通道与 gamestate

已完成 C2S/S2C 独立明文头、native XOR、可靠窗口和有界 gamestate；目标使用保留位池而非旧 Huffman 假设。分片、可靠重发仍待补齐，当前正文足够小，尚不需要分片。

gamestate消费入口 `0x46dea0`、三类configstring列表 `0x46db20` 已复核并实现。资源CRC按 [NET_CONSTANTS_CRC.md](NET_CONSTANTS_CRC.md)计算，真实客户端进度1～6完成；CS2游戏标识 `cod` 的缺失错误已修复。AlphaBuild 可继续提供快照线索，但目标结构与常量编号必须独立确认。

当前训练固定 mp_afghan、war，通过显式准备文件绑定，并验证已捕获的mmstart字段，不执行控制台文本。尚未覆盖完整server规则或多地图；验收依据为客户端实际CRC、初始化进度和状态9，暂未收到后续native ACK3。不要扩大成已可玩。

## 2. 单客户端出生

恢复目标 entity/playerState/netfield 表及 delta 编码，再准备有效的初始 playerState、出生位置、角色/武器索引和持续快照。资源编号必须来自 3.3.3.3；上传的 stats 仅为会话材料，不能直接替代服务器校验后的装备模型。

验收：实际进入场景、看到 HUD/武器、可移动和转向、快照 ACK 持续推进、断开/重进可重复。地图背景图和“设置游戏”均不算出生完成。

## 3. 训练 AI 或剧情首关

基础移动需要真实地图碰撞、服务端时间推进、命中/伤害与实体生命周期，再添加 Bot 控制或任务脚本。优先做一名可交互敌人的完整链路，逐项验证移动、攻击、死亡/重生。

剧情首关仍有独立的分配前置缺口：未发 21/1 时如何取得实时 security key；该路径现在明确失败，尚未复用训练的分配结果。待 key/地图/脚本来源核实后再决定它与训练哪个更快可玩。

## 4. 双客户端

恢复房间成员加入、跨大厅连接的定向推送、每个席位 admission、ready/cancel 与房主生命周期。世界状态按玩家共享，各连接的 DTLS/序号/重放窗口独立。最后验收同图互相可见、移动/伤害同步、断线重进和旧会话撤销。

每一步都分别保留合成回归测试、真实客户端抓包与界面证据，并按可验收阶段提交 Git。当前暂无 Rust 世界模拟循环、地图碰撞加载或 AI，不能把传输 bootstrap 已完成扩大为“服务端已完整联机”。


## 5. 接续校正

最新入口为 [NATIVE_CHANNEL_PROGRESS.md](NATIVE_CHANNEL_PROGRESS.md)。已实测真实客户端接受 sequence=1 的 EOF 和 sequence=2 的空可靠命令，返回 message_ack=2 / reliable_ack=1。

原型实验 client-1791312016136 未成功；当时缺少明文 client-command ACK、native XOR，并沿用旧版 configstring/EOF 编号，因此不能把失败归因为配置项不足。该旧最小原型已删除，当前发送路径使用经实测的新编码器。

上述gamestate步骤已完成，旧最小原型已移除。下一步直接进入实体/playerState：首个有效快照必须通过完整delta解析，不能仅发送opcode和EOF。需要同时建立服务端时钟及有界重发，并继续验证进图与可操作性。
