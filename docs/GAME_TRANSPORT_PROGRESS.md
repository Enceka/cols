# 游戏服分配与 UDP 通道

更新：2026-10-07。目标仍为 CODOL 3.3.3.3，EXE SHA-256 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。

已实测：人机训练分配、mmbook/mmstart、随机 P-224 握手、双向加密 DATA、challenge/connect、32 KiB stats 上传确认与加载心跳均通过。客户端开始发送序号递增的游戏通道数据包，当前停在“设置游戏”，等待尚未实现的 gamestate/快照。没有出生、Bot AI 或完整对局。启动阻塞已恢复，见 [CLIENT_STARTUP_RECOVERY.md](CLIENT_STARTUP_RECOVERY.md)。

下一阶段实施顺序见 [GAMEPLAY_PLAN.md](GAMEPLAY_PLAN.md)。

## 分配与开局控制

1. `21/5 query=3` 是找同伴，实验端点返回空列表，客户端自行建单人 channel。
2. 准备后 `query=1`（仅 build 条件）是查询游戏服务器。返回真实已绑定的 UDP 端点，而非没有接收端的地址。
3. 查找结果比创建描述多 Blob[8] security ID 和一个 UInt32 基类字段；目标解码 `0xa55700 → 0x701db0`。地址为 37 字节 commonAddr，host UID 使用独立的 `1000001`。
4. 客户端向 host 发 `6/14`：UInt64 recipient、Blob（原始 LE32 文本长度 + ASCII 文本）。`mmbook <playlistVersion> <base64 ID> <0|1>` 回复 lobby kind=2、event=21（GLOBAL_INSTANT_MESSAGE），同样长度封装 `mmbook accepted`。目标事件枚举 `0xcd6d80`。
5. 随后收到 `mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0`。只保存元数据，不把收到的命令交给 shell/控制台执行。只有 booked + start requested 后才允许 INIT。

Admission 按 title/UID/登录 seed 绑定；security ID 随机生成，密钥来自该会话实时 `21/1` 描述中的 16 字节 security key。重试保持 ID；新登录、key 变化、退出/删除房间或到期撤销旧分配。有效期不超过票据与 930 秒中的较早者，最多 32 个 admission/peer。尚未在异常大厅断线时立即撤销，仍由上述条件清理。

剧情首关之前实测没有 `21/1`，目前明确返回 `game_allocation_requires_live_key`。后续需要恢复这个路径的密钥来源，不能用零密钥或 AUTH key 冒充。

## 3.3.3.3 握手与 DATA

| 报文 | 本轮观察 |
| --- | --- |
| NAT probe | 29 字节，type=13/version=2；回复 type=12 |
| INIT | 16 字节：6 字节 header、remote tag u16、security ID[8] |
| INIT_ACK | 38 字节；COOKIE_ECHO 原样回送，服务器逐字节验证 |
| COOKIE_ECHO | 189 字节：header[6]、INIT_ACK[38]、commonAddr[37]、ID[8]、public[100] |
| COOKIE_ACK | 114 字节：header[6]、server public[100]、ID[8] |
| DATA | type/version/tag/counter 共 6 字节，MAC[8]、encrypted length u16、加密区、可选明文尾部 |

ECDH 曲线 secp224r1。公钥为 LibTomCrypt DER（包含 bit-string、curve-size=28、X/Y 整数）补零到 100 字节。验证坐标和曲线；服务器每次新 INIT 随机生成私钥，相同 INIT/cookie 重传复用原回复。

目标 `0xa7af90` 的 KDF：`K = SHA1(sharedX[28] || securityKey[16]) || 00000000`，24 字节。`0xb2ced0/0xb2cf50` 的算法注册字符串 `0xbf16f4` 为 sha1，摘要长度 20。AES 使用 K 的前 16 字节，CTR 计数为小端。

目标 DATA 解码 `0xa7fda0`：

- IV = `SHA1(LE32(完整序号) || LE16(目标 tag))[0..16]`。
- MAC = `HMAC-SHA1(K[24], wire[0..6] || wire[14..])[0..8]`。**长度字段及明文尾部均参与校验**。
- 只解密 encrypted length 指定的字节，尾部认证但不加密。
- 16 位 wire counter 还原成完整 32 位序号；目标 `0xb2da20` 为 32 包防重放窗口。认证失败不消耗序号，发送计数耗尽后拒绝继续发送。

这与 AlphaBuild 1.1.1.13 的 Tiger/AES-192/旧 IV 规则不同。之前自洽的收发 roundtrip 测试不足以证明互通，因此新增 Python hashlib/hmac + PyCryptodome 独立生成的合成向量，覆盖跨 AES 块、明文尾部及完整序号。

## 实验记录

- `client-1791297085529`：分配接受，客户端反复发送未实现的 6/14，定位 mmbook。
- `client-1791299964181`：booking/mmstart 接受，加载地图并发出 NAT/INIT。
- `client-1791305078332` / `client-1791305698641`：COOKIE_ACK 接受，旧 DATA 算法校验失败；随机与诊断固定私钥都失败。固定私钥不能解决版本差异，已移除运行时用法。
- 离线核对最后一次抓包：上述 SHA1 KDF + 含长度字段的 MAC 匹配；AES128 解密得到 `getchallenge 0 "<当前 UID 的十六进制>"`，以 NUL 结束。
- **`client-1791307705995`**：新 Rust + 随机 P-224 私钥真实验证，前 30 个 DATA 全部 MAC 通过，无握手错误，明文包括 getchallenge 与 2 字节连接维护数据。截图 `ui-1791307737897.png` 为“荒漠边陲 / 等待连接”。后续仍因未回复 challenge 超时。

完整原始包、私钥相关诊断材料和账号数据只在忽略目录 `research-output`/`client-work`，不要加入 Git 或在日志输出密钥。

## 运行与下一步

```sh
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --prefix-name wine10-recovered-20261007 --seconds 900 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun --rust-game \
  --local-services --trust-lab-cert --quiet-network
```

`--rust-game` 映射 `serve-auth --game-bind 127.0.0.1:39004`，要求本地 services 与 lobby；当前只允许 loopback。抓包在本次实验的 `auth-captures/auth-*/game`，event.json 区分 MAC 通过、错误及明文长度。

下一步恢复内层 netchan、gamestate/snapshot、场景出生与 AI；多客户端成员加入、定向广播和每席 admission 尚未实现。先核实目标消费者，不能直接用旧参考的 protocol/shortversion 常量。

本阶段验证：fmt、Clippy、66 项 Rust、13 项 Python 通过。新增测试覆盖独立加密向量、防重放/乱序/计数耗尽、P-224 cookie/key exchange、分配过期与密钥轮换、mmbook 身份隔离、真实 UDP admission gate 与 INIT 重传。

## 历史阶段：7ee4480 的 challenge 回复与复测限制

提交 `10e8022` 保存了分配、booking 和已实测的加密通道。之后增加 `game_connection.rs`：只向已认证 DATA 中、UID 与 admission 一致的 `getchallenge 0 "<16位 hex UID>"` 回复随机正整数 `challengeResponse`，相同 association 重试复用 challenge。目标 0x466a55 读取 argv(1) 写连接对象 +0x138，随后由 challenging(3) 转 connecting(4)。构造 connect 的目标函数 0x465c00 使用 `connect %s "%s"`（字符串 0xc612bc）；challenge 字段名在 0xc61214。未发送 connectResponse 或伪造世界状态。

67 项 Rust、13 项 Python、fmt/Clippy 通过；真实 UDP 集成测试走 admission gate→INIT→cookie→双向加密 challenge/retry→撤销。它是模拟客户端测试，不代替 CODOL 实测。测试等待初次拒绝事件完成后才开 admission，避免异步抓包写盘延迟造成时序误判。

本轮 `client-1791308194533`、`client-1791308318514`、`client-1791308480466`、`client-1791308564019`、`client-1791308801999` 没有到达可见游戏界面或发出游戏 UDP，不能用于评价 challenge 回包。最初一次手动终止 harness 误用了 SIGTERM，之后已从哈希核对的备份恢复研究副本的临时证书；后续使用 SIGINT 并确认 finally 恢复证书。原始客户端未改动。

macOS 采样发现研究 prefix 的 Wine server 阻塞于 `req_create_file → open_fd → open`，早先还残留 IIPSHostApp。核对 cwd 后终止了研究进程，按 prefix 设备号/inode 隔离了两个无持有者的 `/private/tmp/.wine-501/server-100000f-5fd6ec` 运行目录（保留 `.stale-codol-*` 备份）。重新启动后仍在 IIPS 加载阶段阻塞；不能仅凭残留进程断定全部根因。未重建 prefix 或重置玩家档案。采样保留为 `research-output/wine-start-sample.txt`、`game-start-sample.txt`、`wineserver-blocked-sample.txt`。

该阶段曾需要优先恢复 IIPS；现已解决，见下节。历史建议：确认新客户端可见且已完成 AUTH，再重测 challenge；不要连续复用一个仍卡在文件打开的 Wine server，也不要把零 UDP 的实验记成协议失败。当前运行命令无需改动，新增 challenge 路径已随 `--rust-game` 启用。

## 2026-10-07：完整连接前置流程实测通过

### connect 身份与版本绑定

真实请求为 `connect <16位 hex UID> "\\key\\value..."`，没有末尾 NUL；userinfo 含 name、xuid、natType、rank、2490、checksum、challenge、statver 等。Rust 校验外层 UID、userinfo 的 xuid 都等于 admission 身份，challenge 必须是当前 association 已签发值，`2490` 必须等于实时描述的 build。拒绝重复字段、缺失必需字段、超长/不配对字段和控制字符；不执行文本，也不以玩家自报 rank 作为授权。

Admission 新增 build 绑定，build/key/登录 seed 变化撤销旧分配。目标 0x466b24..0x466c22 在 connectResponse 后只初始化 netchan（0x5a5420），进入状态 5 并开始 stats 上传；未读取旧参考的 info-string。因此回复为 OOB `connectResponse`，没有照搬 1.1.1.13 的 shortversion/protocol 常量。

### 目标 stats 分块格式

完整明文：`ffffffff || "stats" || 00 || chunk:u8 || compressed_total:LE16 || fragment`。**这个位置是 chunk + 压缩长度，不是旧参考的 qport + chunk。**

目标 0x6d80d0 决定分块数与重发索引，0x6d81a0 写索引、长度和片段：每片至多 0x4d6=1238 字节；compressed_total=0 表示原始 32768 字节，否则按指定总长度重组 zlib，输出须恰好 32768 字节。最大 27 片，对应客户端 mask 0x7ffffff。服务端 `game_stats::Upload` 有界重组，接受相同重传，拒绝冲突重传/中途改变长度、截断、超界索引、额外 zlib 尾部和过量展开。

回复 `statResponse <实际缺失位图>`；只有全部片段收到且完整解压验证后才返回 0。不是收到任一 stats 就固定 ACK 0。真实训练样本压缩长度为 1192/1194 等，单片即可完整上传。之后客户端不再反复重传 stats。

当前只把上传作为会话内的原始材料，不写回大厅持久档案或信任其中的等级/装备。首个真实样本内部首 DWORD 与 `CRC32(bytes[4..])` 不一致：目标 0x6d81a0 路径先调用 0x6d7d80 计算 CRC，再调用 0x6d8a70 调整装备，可能有关；尚未确定原因。现有校验保证 DTLS MAC、分块完整性、zlib 流完整性和 32 KiB 长度，不宣称验证了 stats 内部业务含义。

### 心跳与内层游戏包

stats 完成后，目标处于状态 6 时发送 OOB `heartbeat`；仅在 stats 完整接收后回复 `heartbeatresponse`。目标 0x466c30 分支检查本地地图加载工作线程（0x4640a0），完成后继续进入游戏通道。

`client-1791310514572` 已从 heartbeat 转为序号递增的非 OOB 包，首三个明文分别为 `0100000000000000000000000037`、`0200000000000000000000000037`、`0300000000000000000000000037`，后续还出现 64 字节包。服务端目前只认证/解密和保存这些包，尚未恢复内层消息解析或发送 gamestate，不能据此声称快照同步。

### 已停止实验的证据

| 实验 | 进展 | DATA MAC 通过 |
| --- | --- | ---: |
| client-1791309533210 | 文件替换后恢复大厅，challenge 回复接受并收到 connect | 78 |
| client-1791309800878 | connectResponse 接受，界面“同步游戏设置”，收到重复 stats | 1178 |
| client-1791310250844 | 完整 stats ACK 接受，转 heartbeat，界面“设置游戏” | 119 |
| client-1791310514572 | heartbeatresponse 接受，产生递增序号游戏包 | 145 |

以上实验 event.json 均无错误，停止后临时证书均已恢复。截图与原始包只留在忽略目录。71 项 Rust、13 项 Python、fmt/Clippy 通过；新增覆盖 connect 身份/build/challenge、防重复字段、真实 UDP connect 回复、stats 重组/冲突/解压边界以及未完成 stats 禁止推进心跳。


## 2026-10-07 gamestate 原型失败边界

在 `client-1791312016136` 中尝试了 152 字节的 gamestate 原型（当时错误使用 opcode 2，后来静态确认目标 opcode 3 才进入 0x46dea0 gamestate 解析）。外层 DATA 已认证，客户端没有接受；截图 `ui-1791312268118.png` 显示“服务器连接超时”。已移除自动发送，避免后续实验继续下发未经验证的空壳状态。目标完整 gamestate 仍需恢复 configstring bank、资源纯校验和可靠命令内容；参考包报告的 743 项/2992 字节不能直接移植到 3.3.3.3。

目标消费者静态地址：内层分派 0x46e6a0；gamestate 解析 0x46dea0；configstring 字符串 `CA_GAMESTATE` 0xc61bb8；更新提示 `gamestateupdate %d` 0xc61d90。目标内层 stream 还调用 0x46bf80 native XOR，key 使用关联 outgoing sequence/challenge 状态并滚动命令字节；当前最小原型未做该 XOR，这也是失败实验的待排查因素，不能把失败归因于 configstring 缺失。`game_wire::xor_stream` 仅有离线可逆测试，未接入生产发送。

内层观测器现已接入但只做验证和记录：读取 LE32 native sequence，拒绝 ACK 位和回退序号，记录丢失包数，并按目标分派表标记 0/1/2/3/5/6 为 Snapshot/ReliableServerCommand/ReliableClientCommand/Gamestate/End/Configstring；未知码保留为 Unknown。DATA 外层通过后，事件文件增加 `inner_kind` 与 `inner_dropped`。不对输入执行 XOR 解密，因为 0x46bf80 的 key 还依赖连接对象 `+0x40148`、challenge 状态 `+0x138` 和全局 `DAT_0796f2d8`；其中 `DAT_0796f2d8` 由 0x46d830 的运行时函数计算，不能用 DTLS key 替代。

Rust `game_wire::NativeXor` 已把两个方向分开：`incoming_xor(sequence_byte, challenge_byte, command, data)` 对应 0x46bf80；`outgoing_xor(..., feed_byte, ...)` 对应 0x46bf00，额外 feed_byte 对应目标 `DAT_0796f2d8`。状态跨分片保留 command cursor 和奇偶 offset，在 C 字符串 NUL 后重启；测试覆盖跨调用分块、方向错误和可逆性。它仍未接入真实回复，因为当前 admission 没有经目标验证的 command 表/feed 值。

服务端方向另有 `server_outgoing_xor/server_packet`：保持 LE32 outgoing sequence 明文，body 使用 `challenge ^ (sequence as u8)` 与 last client command。这个公式来自 AlphaBuild `SV_Netchan_Encode`，独立于目标客户端 0x46bf00 的 feed 版本；对应测试已覆盖 sequence 保持明文和逆变换。当前仍只作为编解码地基，不自动发送 gamestate。

可靠窗口基础已加入 `game_wire::ReliableCommands`：256 项、每项最多 1023 字节；ACK 只能单调前进到已发送序号，确认后释放旧 ring 项，窗口满时拒绝继续排队。AlphaBuild 的 gamestate 发送顺序是可靠 ACK → 待发送可靠命令 → gamestate，同一个 native netchan 包发送；这为后续目标版本的 command ring 接入提供边界，但尚未把猜测的命令内容发给客户端。

## 2026-10-07 可靠消息解析入口

目标函数 `0x467520` 在连接状态大于 6 后调用 `0x5a6160` 处理 native stream；`0x46e6a0` 是其消息码分派循环。目标函数 `0x46e470` 读取一个 LE32 序号、LE16 长度（必须小于 `0x3fd`）及正文，并按序号保存到客户端 command ring；这对应可靠 server-command/config 更新的接收侧。`0x46e3b0` 读取 LE32 sequence 后再读取一段最多 0x400 字节字符串，保存为另一类可靠命令。两者都在 opcode 分派后的独立路径中，不能把所有正文统一当 configstring。

客户端读取器 `0x5a1180/0x5a1630` 保持 bit cursor、环形缓冲和跨消息边界；服务端参考 `SV_Netchan_Encode` 则在发送前保留前 4 字节序号，并使用 `challenge ^ outgoingSequence` 与 `lastClientCommandString` 对后续字节做 native XOR。Rust 当前已经有对应的 server packet/XOR 和可靠窗口原型，但没有将任意 command 放入 ring，也没有根据未识别的目标 opcode 发送可靠消息。

下一步应从真实客户端首个 14 字节包中恢复 XOR 后的 native opcode/ACK（需要 command ring 初值），再用一个无副作用的 reliable command 做确认；只有确认成功后，才构造完整 gamestate。直接发送大块配置仍会掩盖 opcode、ACK 或 command-string 错误。

服务端观测器现在对长度至少 14 的 C2S native 包优先解析 `ClientHeader`，记录 `client_server_id`、`client_message_ack`、`client_reliable_ack`，并用其 sequence 计算丢包；不再把这类包的 server-id 字节误标记成 opcode。较短的未知包仍走通用 sequence/opcode 观测。字段只写入 event.json，不更新可靠 ring，也不改变连接状态。

短时真实复测 `client-1791335109328`：73 个 C2S native 包的 header 稳定为 `server_id=0, message_ack=0, reliable_ack=0`，每包 14 字节，剩余一个 body byte；外层 MAC 全部通过且无服务端解析错误。该 body byte 不是完整 opcode 消息，当前只记录为 `client_body_first`，不把它解释成 ACK/EOF。客户端仍停在加载等待，说明下一步需要服务端先发送合法 S2C gamestate/可靠包，才能让客户端产生非零 ACK。

观测路径现用 association challenge 和 acknowledged command 槽对 C2S body 做目标 XOR 解码；初始 `reliable_ack=0` 使用空命令槽。事件新增 `client_opcode`，真实样本将持续验证是否为 clc_EOF=3。ACK 数值暂不推进 reliable ring，避免服务端尚未发送命令时接受伪造确认。

真实复测 `client-1791336561121`：108 个外层 DATA 中 93 个 C2S native 包完整解码为 `client_opcode=3`（clc_EOF），且 93 个 header 都是 `(server_id,message_ack,reliable_ack)=(0,0,0)`；其余包为 OOB/握手阶段。没有发送任何 S2C gamestate，因此 ACK 保持 0 是预期结果。这个结果确认 challenge XOR、13 字节 C2S header 和 EOF 解码方向均正确。

### 首个 S2C EOF 探针

Connection 现在只在第一个已连接的 C2S `clc_EOF` 上回复一个 native S2C EOF：`sequence=1`、`clientCommandAck=0`、正文 opcode=6，随后不再重复发送，也不发送 gamestate。真实复测 `client-1791337276586` 的 `0016-reply.bin` 外层包长 25；下一个客户端包 `0017-plaintext.bin` 的 header 从 `(server_id,message_ack,reliable_ack)=(0,0,0)` 变为 `(0,1,0)`，之后保持 message_ack=1。客户端接受该包且没有 MAC/解析错误，证明 S2C 两个清晰 LE32 字段、`challenge ^ sequence` XOR 和 opcode=6 组合正确。

该探针仍停在加载等待，原因是 EOF 只推进 message ACK；下一步的最小安全实验是发送一条真实可靠 command（使用已确认的 ring 序号和 client ACK），观察 reliable_ack 是否推进，再恢复完整 gamestate。当前不会用 EOF 探针伪装出生或对局。

方向边界的独立向量已加入：C2S 样本 `01 00 00 00 00 00 00 00 00 00 00 00 00 76` 解析为 sequence=1、serverId=0、messageAck=0、reliableAck=0、单字节 body=0x76；S2C writer 保持 sequence 与 clientCommandAck 两个 LE32 字段明文，再只对正文 XOR。这个边界与目标 `0x467520` 接收流程及参考 `SV_Netchan_Encode` 一致。

结合目标入站 XOR `0x46bf80` 和参考 clc 表，第二个向量进一步解出 C2S keepalive：实验 `client-1791312016136` 的 challenge 低字节为 `0x75`，body `0x76 ^ 0x75 = 0x03`，即 `clc_EOF`。这解释了客户端在等待阶段持续发送 14 字节包：它们是合法的空客户端消息，而不是 gamestate opcode。Rust 已加入该向量测试；真实服务端仍只记录，不回传猜测状态。

目标 `0x5a5720` 的重组器还显示了版本特有的分片阈值：普通路径比较 `0x4d8`，关联类型为 2 时比较 `0x482`；超过阈值会进入内部片段 ring，而不是简单把完整正文当一个 UDP 包。AlphaBuild 的旧 `CODOL_FRAGMENT_SIZE=1162 (0x48a)` 不能直接套用。Rust 当前 server packet 明确拒绝超过阈值的正文，防止向目标发错片段；下一步需从目标实包恢复片段头字段后再实现。

feed 来源已进一步确认：0x46d830 在调用 0x4f9850 前把 `ECX=3`，即读取 configstring index 3；随后 0xaf6e68 调用十进制 `strtol`（base=10），结果写入 `DAT_0796f2d8`。因此服务端未来应从 gamestate 的 server-id/configstring 3 生成 `feed_byte = parsed_u32 as u8`，不能从 challenge、DTLS key 或用户 UID 派生。Rust 已加入 `native_feed_byte` 纯函数和边界测试；command ring 与 `+0x40148/+0x138` 的运行时值仍待目标包确认。

真实内层包在 heartbeat 后是 14 字节：`LE32(sequence) || 9 个零字节 || 末尾值`，不同实验末尾分别观察到 `0x37` 和 `0x76`；这不是可直接当作 gamestate 的明文。目标接收器 0x46e6a0 使用 0x5a1180 读取四字节序号、再按消息码分派；0x46dea0 的 gamestate 路径会读取可靠确认号、检查 configstring/服务器校验并推进状态。此前 152 字节原型没有经过这些状态前置，不能单独用它推断配置内容不足。
