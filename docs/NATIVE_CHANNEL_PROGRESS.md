# 3.3.3.3 native 通道：最新接续

2026-10-07。本文替代 GAME_TRANSPORT_PROGRESS 后半段中的旧方向/标记推测。目标 EXE hash 保持 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。

**已实测可靠消息接收确认，尚无 gamestate 或出生。** 实验 `client-1791338632345`：151 个 UDP 事件无错误；0016 收到初始 EOF，回复 native 消息1；0017 客户端 message_ack=1，回复 native 消息2（可靠命令序号1，空字符串）；0018 开始 message_ack=2、reliable_ack=1，连续114个 C2S 包保持该确认。玩家配置没有被修改；最终仍因缺少 gamestate 等待超时。

## 两种方向的真实格式

| 方向 | 不做 native XOR 的头 | 正文 XOR 初值 | 命令槽 |
| --- | --- | --- | --- |
| C2S | sequence LE32 + serverId u8 + messageAck LE32 + reliableAck LE32（13字节） | low8(challenge XOR messageAck) XOR serverId | 服务端发送且客户端已确认的命令，索引 reliableAck |
| S2C | sequence LE32 + clientCommandAck LE32（8字节） | low8(challenge XOR sequence) | 已接收且在本包头确认的客户端命令 |

双方都在 DTLS DATA 内再做上述 XOR。目标 0x4602a0 构造 C2S 的9字节应用头，0x46bff0/0x46bf00 只 XOR 其后内容，0x5a5720 添加序号。目标 0x467520 读取 S2C 两个 LE32，0x46bf80 XOR 后续正文。ACK 的最高位标记 zlib；当前只写未压缩包。

`DAT_0796f2d8` 是 configstring3 经十进制解析的 serverId，参与 **C2S** 初值；不是独立随机密钥，也不参与 S2C 初值。gamestate 尾部的 checksumFeed 是另一字段（连接对象+0x13c），不可混淆。旧 `incoming_xor/outgoing_xor/server_outgoing_xor` 的方向命名和4字节 XOR 偏移已被 `open_client/server_packet` 取代。

## opcode 和位池

- S2C 分派 0x46e6a0：0=snapshot，1=可靠字符串（0x46e3b0），2=可靠二进制（0x46e470），3=gamestate（0x46dea0），5=另一请求路径（0x46e620，语义待恢复），6=EOF。**2不是客户端命令，6不是 configstring。**
- Gamestate 内部只有4进入 configstring 读取0x46db20，6结束该段，再读 clientNumber 和 checksumFeed。最终外层消息同样需要 EOF6。
- C2S 构造0x4603e0/0x4606a8：opcode占3位，2=客户端可靠字符串（LE32序号、NUL字符串），3=EOF。0/1为移动路径，当前未实现。
- 3-bit opcode 使用独立位池。插入整字节序号/字符串时，位池中的剩余位继续保留；新位池字节在当前字节游标处预留。目标读取0x40c370/0x5a13a0；不能将其当作连续普通 bitstream 或旧 Huffman 数据。

## 当前连接状态与边界

必须先完成 stats，才处理 native 包。当前只接受 serverId=0、单调递增客户端序号、未超出已发送消息范围的 message ACK，以及合法的可靠 ACK；未支持的客户端命令明确失败，不执行也不冒充确认。完整解析成功后才提交序号/ACK。

S2C探针1：`sequence=1, clientCommandAck=0, body=[6]`。

S2C探针2：收到 messageAck=1 后，`sequence=2, clientCommandAck=0, body=[1, LE32(1), 00, 06]`，即序号1的空可靠字符串和EOF。`reliable_ack=1`证明客户端收到该序号，不证明任何游戏逻辑执行。没有收到客户端可靠命令，所以 clientCommandAck 必须仍为0。

此前未实测提交 f820c9e 的探针错误地设置 clientCommandAck=1、漏写EOF、发送了会改变 serverId 的 `d 3 1`。本轮已移除该版本，采用无配置变更的空命令，使用真实 C2S ACK 验证。

可靠窗口按完整序号保留256槽；确认后的当前命令仍供后续包 XOR 使用，不能 ACK 后立即删除或对不存在槽静默返回空字符串。queued、sent、acknowledged 分开；观察字段使用当前已知槽解码，不再对任意 reliableAck 使用空槽。

当前两个探针是有界、一次性诊断。丢包重发、完整服务端可靠命令发布、客户端命令执行、移动解析和 native 分片都未完成。不得把 transport/reliable ACK 扩大为游戏已可玩。

## 需要纠正的旧实验解释

152字节原型超时没有证明 configstring 数量不足，因为当时消息头、XOR和opcode同时不匹配。服务端只能证明发送了 DATA，不能以自身编码成功声称客户端验证了它的 MAC；正确证据是后续 ACK 推进。

`0x5a5720` 是发送路径，`0x5a5960` 才是重组接收器。发送阈值表达式 `(type != 2 ? -86 : 0)+0x4d8`：type2=1240，其他=1154；0x482=1154，0x48a=1162。目标分片发送0x5a55f0使用0x482。旧文档倒置条件且混淆十六进制换算；目前未根据这些错误结论发送分片。

## 后续 gamestate

目标0x46dea0读 serverCommandSequence、两个NUL字符串、net-constant CRC（须与0x02df7d90一致），再读configstring段、clientNumber和checksumFeed。0x46db20有普通字符串列表及另外两个10-bit计数的紧凑列表，容量0x1b92=7058，不能直接复制Alpha的6519项格式。

先只读提取目标运行时 net-constant 资产/CRC及configstring类型，再依据真实资源生成消息。不要用零CRC或关闭校验代替恢复来源。

验证：fmt/Clippy、77项Rust、13项Python。状态测试覆盖stats前拒绝、未发送ACK拒绝、无EOF/错误序号不推进、探针保持client ACK=0、完整EOF、可靠ACK推进。原始包位于忽略目录，账号和密钥不提交Git。

## 网络常量只读采集

`tools/client_state.py --net-constants` 已加入固定资源提取模式：扫描当前 asset hash bucket 的 type44（netconststrings）头，导出其 name/category/source/count 与原始字符串字节，以及 NCS 类型布局、初始化状态、common/current CRC 和已经链接的资源列表。只使用 ReadProcessMemory，不调用游戏函数；查询中没有 AUTH/DTLS 密钥。解析错误会保留为 `error`，summary 的 `asset_errors` 非零时不能当成完整资源集。

编译与采集：

```sh
python3 tools/client_state.py --build --prefix-name wine10-recovered-20261007
python3 tools/client_state.py --experiment research-output/gamehub/实际实验/experiment.json --net-constants
```

编译必须在该 prefix 的客户端实验停止后进行；普通采集则要求对应实验仍在运行。新 `--prefix-name` 只用于 build，沿用 ownership marker 与路径校验。`ReadClientState` 普通模式同时新增 connection_state、server_message_sequence、client_command_ack、server_command_sequence 和 net_constants_crc，便于确认接收状态。

`client-1791339446435/net-constants-1791339501447.json`：大厅 state0、NCS initialized1、current CRC `0x396299aa`、common CRC `0xcdb31851`、26个链接资源。旧采集器按 flags 过滤资源只找到18个空 level 表，此过滤已撤销，不能据该数量宣称地图常量缺失。同实验 `net-constants-1791340054465.json` 在 state7 时读取 initialized0/current CRC0/linked0，说明加载状态存在未初始化阶段；不能把这个0或大厅 CRC 直接下发。

修正哈希遍历后的 `client-1791340243685/net-constants-1791340629268.json`：大厅 state0 下43条 type44记录，其中42条按结构解析，1条默认 `void/category21/source4` 被记录为错误；26个linked记录。含 common、localized、level 资源和运行时生成的 codinfo/net dvar 表。校验步骤尚未完成：按当前 linked 顺序简单拼接字符串的 CRC 与实际 CRC 不同，不将读取值硬编码为地图的合法校验值。

目标来源：类型名表0xce0ea0确认type44；0x585000按source、category、名称后缀类别与名称排序；0x584d80去重；0x584ed0将字符串字节送入CRC32链；0x5850b0组合运行时/关卡表。`0xc57118` 是21组(category base, capacity, reserved)布局：动态dvar名从24开始、值从344开始，容量各320；武器从5443开始、容量900；这些不同于Alpha布局。

下一步采集一次修正工具的 **state7地图资源集**，恢复排序、去重和公共seed来源，得到能独立复算的CRC，再编写目标gamestate。当前只有大厅修正提取完整走通，不能声称已取得可用的 mp_afghan CRC。
