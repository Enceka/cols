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


## 5. 最小 gamestate 试验结论

本轮用 AlphaBuild `CODOL_WriteGamestate` 的目标顺序（opcode 2、command sequence、map、zone type、CRC、opcode 3 configstrings、client number、checksum feed、opcode 5）构造了 152 字节原型，只包含 map/mode/server ID/systeminfo 三项。Rust bit writer 的低位优先顺序与参考 `codol_msg.c` 一致。

真实客户端在收到该原型后没有进入 gamestate ACK 或场景，约 30 秒后弹出“服务器连接超时”。服务端原始事件显示该响应已经通过外层 DATA MAC；后续客户端仍发送少量 14 字节内层包，未发送可确认的 gamestate ACK。结论是 configstring 数量、内容、checksum feed、完整 `mp_afghan` 资源/纯校验或其他可靠命令至少有一项必需，不能把原型接入默认运行路径。

`game_wire::minimal_gamestate` 和 4 字节 sequence 原型保留为离线编码测试，但当前 Connection 不自动发送它。下一步需要恢复目标版本完整 configstring bank（AlphaBuild 的 743 项/2992 字节仅作为旧版本结构参考），再用真实 ACK 验证。

静态解析进一步确认：客户端内层消息分派在 0x46e6a0，消息码 0/1/2/3/5 进入不同解析器，消息码 6 读取四字节 client number 后进入快照/可靠流路径；gamestate 解析器入口为 0x46dea0，结束标记和 configstring 读取不能从旧版 opcode 表直接猜测。目标连接后对服务端内层字节还会执行 0x46bf80 的 native XOR：初始 key 来自关联的 outgoing sequence/challenge 状态，并按命令字符串字节和奇偶位滚动。它与 DTLS DATA 密钥独立；Rust 已保留可逆 `xor_stream` 原型，但未接入发送路径，因 key/command 状态尚未被目标抓包确认。
