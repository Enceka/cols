# 游戏服分配与 UDP 加密通道

本版已完成分配、booking、P-224 握手、加密 DATA、challenge／connect、stats 上传及原生游戏通道接入。实际场景、权威对局与共享多人范围见[玩法](GAMEPLAY_PLAN.md)和[多人验收](MULTICLIENT_ACCEPTANCE.md)。

## 分配与准入

准备后的 `21/5 query1` 返回实际已绑定游戏端点。查找记录包含 commonAddr[37]、security ID[8] 及目标版描述字段；不能原样回显创建请求。`6/14` 的 `mmbook` 与 `mmstart` 完成后才接受 INIT。文本只解析明确控制字段，不交给 shell 或通用控制台。

准入绑定已认证 title／UID／登录 seed、build、频道和实时 security key。重试复用当前分配；身份、密钥或 build 改变、退出／删除及过期撤销旧准入。持续游玩对验证通过的活动连接续期；结算释放房间进行中约束，保留旧连接发送结算，新开局生成独立分配。

剧情采集通过 `--story-capture-map` 提供独立随机 ID／key 和本版 `coop` 入口，不发送 TDM 世界。实际首关握手与资源初始化见[剧情资源](STORY_RESOURCE_CAPTURE.md)。

## 3.3.3.3 握手与 DATA

| 报文 | 本版格式 |
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

这与 AlphaBuild 1.1.1.13 的 Tiger/AES-192/旧 IV 规则不同。独立 Python hashlib/hmac 与 PyCryptodome 合成向量覆盖跨 AES 块、明文尾部及完整序号。

### connect 身份与版本绑定

真实请求为 `connect <16位 hex UID> "\\key\\value..."`，没有末尾 NUL；userinfo 含 name、xuid、natType、rank、2490、checksum、challenge、statver 等。Rust 校验外层 UID、userinfo 的 xuid 都等于 admission 身份，challenge 必须是当前 association 已签发值，`2490` 必须等于实时描述的 build。拒绝重复字段、缺失必需字段、超长/不配对字段和控制字符；不执行文本，也不以玩家自报 rank 作为授权。

Admission 新增 build 绑定，build/key/登录 seed 变化撤销旧分配。目标 0x466b24..0x466c22 在 connectResponse 后只初始化 netchan（0x5a5420），进入状态 5 并开始 stats 上传；未读取旧参考的 info-string。因此回复为 OOB `connectResponse`，没有照搬 1.1.1.13 的 shortversion/protocol 常量。

### 目标 stats 分块格式

完整明文：`ffffffff || "stats" || 00 || chunk:u8 || compressed_total:LE16 || fragment`。**这个位置是 chunk + 压缩长度，不是旧参考的 qport + chunk。**

目标 0x6d80d0 决定分块数与重发索引，0x6d81a0 写索引、长度和片段：每片至多 0x4d6=1238 字节；compressed_total=0 表示原始 32768 字节，否则按指定总长度重组 zlib，输出须恰好 32768 字节。最大 27 片，对应客户端 mask 0x7ffffff。服务端 `codol_protocol::game::stats::Upload` 有界重组，接受相同重传，拒绝冲突重传/中途改变长度、截断、超界索引、额外 zlib 尾部和过量展开。

回复 `statResponse <实际缺失位图>`；只有全部片段收到且完整解压验证后才返回 0。不是收到任一 stats 就固定 ACK 0。真实训练样本压缩长度为 1192/1194 等，单片即可完整上传。之后客户端不再反复重传 stats。

当前只把上传作为会话内的原始材料，不写回大厅持久档案或信任其中的等级/装备。首个真实样本内部首 DWORD 与 `CRC32(bytes[4..])` 不一致：目标 0x6d81a0 路径先调用 0x6d7d80 计算 CRC，再调用 0x6d8a70 调整装备，可能有关；尚未确定原因。现有校验保证 DTLS MAC、分块完整性、zlib 流完整性和 32 KiB 长度，不宣称验证了 stats 内部业务含义。

### 心跳与内层游戏包

stats 完成后，目标处于状态 6 时发送 OOB `heartbeat`；仅在 stats 完整接收后回复 `heartbeatresponse`。目标 0x466c30 分支检查本地地图加载工作线程（0x4640a0），完成后继续进入游戏通道。

heartbeat通过后进入带序号的native消息，解析和发送格式见[原生通道](NATIVE_CHANNEL_PROGRESS.md)。

## 原生数据与验证

DATA 内层方向头、XOR、位池、可靠命令和分片见[原生通道](NATIVE_CHANNEL_PROGRESS.md)，世界复制见[快照](SNAPSHOT_PROGRESS.md)。原始 stats 只作会话材料，不作为服务端装备或等级授权。

回归覆盖独立加密向量、cookie／INIT 重传、乱序／防重放、计数耗尽、准入轮换、身份绑定、stats 重组与原生 ACK。原生单客户端和双客户端核心流程已有对应证据；公网 UDP 直连、丢包条件和完整重连恢复未由这些结果证明。
