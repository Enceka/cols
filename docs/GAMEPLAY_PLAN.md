# 从已连接到实际对局

更新：2026-10-07。登录、训练分配、DTLS、challenge/connect、stats 上传确认、加载心跳均已获 3.3.3.3 客户端接受。现在客户端发送有序游戏包并停在“设置游戏”。本文件只规划尚未完成的世界状态部分；协议证据和实验见 [GAME_TRANSPORT_PROGRESS.md](GAME_TRANSPORT_PROGRESS.md)。

## 1. 游戏通道与 gamestate

先确认目标 netchan 的序号、ACK 位、分片、qport 是否存在、压缩范围及 Huffman 表，再实现 Rust 编解码。首批观测为 14 字节 `LE32(seq) || 9×00 || 37`；之后可出现 64 字节包。现在只有外层 DATA 验证成功，不能把这些内层字节直接当成已解析的用户指令。

从目标 `CA_GAMESTATE` 字符串 0xc61bb8、引用 0x46a69d，`gamestateupdate %d` 0xc61d90、引用 0x4459b4/0x445cb1/0x46de54/0x46df1c 继续定位真实消息消费者。这些是字符串引用地址，不是已确认的函数入口。AlphaBuild 的 `codol_netchan.c`、`codol_wire.*` 和 gamestate writer 可提供线索，但 qport、opcode、常量表都要在新版本复核。

为当前训练固定目标地图 mp_afghan、模式 war 建立明确配置；从受控赛单解析所需规则，不能执行收到的 mmstart 文本。恢复 serverinfo/systeminfo、configstrings、资源基线及游戏状态消息，只有实际客户端 ACK 并进入下一连接状态后才计为通过。不要下发随意拼接的空 gamestate 来宣称可玩。

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

原型实验 client-1791312016136 未成功；当时缺少明文 client-command ACK、native XOR，并沿用旧版 configstring/EOF 编号，因此不能把失败归因为配置项不足。代码中的旧最小 gamestate 没有接入发送路径，也不应作为目标格式的依据。

首先恢复运行时 net-constant 校验值、可覆盖 configstring 类型、正文 opcode 3/4/6 与两类 10-bit 紧凑列表。接着验证 gamestateupdate 与目标连接状态8；最后才能进行实体/playerState 和场景出生。
