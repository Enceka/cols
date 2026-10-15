# 原生游戏通道、可靠命令与分片

目标为固定 3.3.3.3；外层加密见[UDP 通道](GAME_TRANSPORT_PROGRESS.md)。gamestate、快照及双向输入已用于实际对局，不以传输 ACK 代替玩法验收。

## 方向头与 XOR

| 方向 | 不做 native XOR 的头 | 正文初值 | 命令槽 |
| --- | --- | --- | --- |
| C2S | sequence LE32、serverId u8、messageAck LE32、reliableAck LE32，共 13 字节 | low8(challenge XOR messageAck) XOR serverId | 已发送且客户端确认的服务端命令 |
| S2C | sequence LE32、clientCommandAck LE32，共 8 字节 | low8(challenge XOR sequence) | 本包确认的客户端命令 |

`0x4602a0`／`0x46bff0`／`0x46bf00` 为 C2S 构造与 XOR，`0x467520`／`0x46bf80` 为 S2C 读取与 XOR。serverId 来自 CS3；checksumFeed 是独立字段，不能作为同一初值。

ACK 高位标记 zlib。服务端先压缩正文再 XOR，客户端先 XOR 再解压；小消息保持未压缩。压缩仍有预算，超大或不可压缩输入拒绝发送。

## 位池与消息类型

S2C `0x46e6a0` 分派：0=快照、1=可靠字符串、2=可靠二进制、3=gamestate、5=未完整恢复的请求、6=EOF。gamestate 内部 opcode4 读取 configstring，6 结束该段，然后读取 clientNumber 和 checksumFeed；外层仍须 EOF6。

C2S 使用 3-bit opcode：0／1 为移动，2 为可靠字符串（LE32 序号和 NUL 字符串），3 为 EOF。位池与整字节游标独立；写入字节不会抛弃尚未消费的位池。该格式不能当成普通连续 bitstream 或旧 Huffman 数据。

## 可靠状态与输入基线

先完成 stats，再接受 native 数据。校验客户端序号、已发送消息范围与可靠 ACK；整包解析成功后提交状态。可靠窗口保留 256 槽，queued／sent／acknowledged 分离，已确认当前命令继续参与 XOR，不能立即删除或对缺槽返回空字符串。

可靠命令有明确业务入口和参数校验，不执行任意客户端控制台文本。资源注册、背包切换、经验与结算按 ACK 门槛推进；窗口满时保留待发批次，避免部分发布和阻塞快照轮询。经验／结算重试见[等级](PROGRESSION_PREPARATION.md)。

移动输入以已确认快照的 PS 为默认 base；可靠命令消息不能替换该 base。具体字段、历史和验证见[快照](SNAPSHOT_PROGRESS.md)。

## gamestate 与资源

`0x46dea0` 消费 serverCommandSequence、地图／类型字符串、NCS CRC、configstring 段和连接尾字段。`0x46db20` 支持普通列表及两个 10-bit 计数的紧凑列表，总容量 7058。

CS0 使用 `mapname`／`g_gametype`，CS2 为 `cod`，CS3 为 serverId，CS6415 为正常时间倍率 `0 0 1 1`。不可变 NCS 类别由实际资源继承，不能重复覆盖。CRC 必须按本图已初始化资源复算，见[网络常量](NET_CONSTANTS_CRC.md)。

## 索引式分片

本版 `0x5a5490`／`0x5a5960` 使用带 FRAGMENT_BIT 的 seq32，随后为 index8、lastIndex8、stride16、length16 和数据。当前 stride=1154，最多 128 片；ACK 与完整 XOR／zlib 负载先组成消息，再拆分。

gamestate 和快照共用发送队列，当前序列发送完之前不发布更高序号回复。回归覆盖乱序、重复、完整重组和解压；原生地图物件、死亡／回放场景已有多分片接收证据。分片 ACK 请求／重传控制尚未实施，当前发送不请求该标志。
