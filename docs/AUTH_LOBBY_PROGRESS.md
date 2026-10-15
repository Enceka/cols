# AUTH 与协议 210 大厅通道

固定客户端 3.3.3.3 已接受 Rust 本地票据并完成协议 210 握手、加密业务交互及大厅初始化。档案、库存、房间和游戏接入是独立业务模块，见[功能状态](REFERENCE_ALIGNMENT.md)。

**AUTH 票据实现**

`crates/protocol/src/auth/ticket.rs` 实现 128 字节小端布局、Tiger/192（Tiger1）、Tiger(LE32(seed)) IV 和无额外 padding 的 3DES-CBC；与 AlphaBuild 的独立 Python/PyCryptodome 生成结果逐字节一致。参考来源及合成参数保存在 `fixtures/protocol/auth-ticket-alpha-vector.json`。

`local_ticket_response` 生成 task 11 / code 700 的 JSON，回显 iv_seed，并使用 Base64 票据。当前两个票据字段采用参考实现的同一密文形式，已被本样本接受并用于后续握手。

运行模式是**本地测试身份**：UID 来自已解析请求，校验配置的 title_id，每次生成新的 24 字节会话密钥，票据有效期一小时。没有复用参考包的固定 UID=1 或固定全局密钥，也不表示已实现腾讯账号认证。

AUTH 与 LSG 共享内存中的 SessionRegistry：以完整的已签发 server_ticket 查找会话，再检查 title_id、iv_seed 和有效期。未签发/过期票据不会得到大厅认证。私有实验记录仍保存于忽略目录，不把真实会话密钥放入公开 fixtures 或控制台日志。

**协议 210 与旧参考的关键差异**

目标客户端在 VA `0x00a610c0` 构造 28 字节 preface：

```text
u32 LE 200
u32 LE 200
u32 LE 210
u32 LE 210
u32 LE client_max_receive   // 实测 0x80000
byte[8] client_nonce
```

VA `0x00a61980` 的响应读取明确要求选定协议 210；此路径不会接受直接回落为 200。服务器第一条回复：

```text
u32 LE 22
u8 0xAB
u8 0x81
u32 LE 210
u64 LE connection_id
byte[8] server_nonce
```

客户端随后发出 AB82。其 body 为：种子字节 7、1 位布尔值、5 位 UInt32 标签、title_id、另一 UInt32 标签及 iv_seed、128 字节 server_ticket，最后带 8 字节 client proof。这里的 bit buffer 按 LSB-first 读取；实测完整帧 153 字节、位缓冲区 139 字节。

Key derivation 已由静态代码和客户端实际证明交叉验证：

```text
transcript = client_preface[8..] || server_hello || client_auth_without_last_8_bytes
salt = SHA1(transcript)
PRK = HMAC-SHA1(key=salt, data=24-byte AUTH session_key)
challenge = HKDF-Expand(PRK, "CLIENTCHAL", 16)
data_keys = HKDF-Expand(PRK, "BDDATA", 72)
```

注意：client-auth 的长度前缀仍使用完整帧长度，但 transcript 不包含最后的证明。`challenge[0..8]` 验证客户端；服务器通过 AB83 发回 `challenge[8..16]`。服务器只有在已签发票据匹配且 client proof 正确时才发送此回复。

关键函数：server-header reader `0x00a61980`、server-proof check `0x00a618c0`、HKDF extract `0x00a5fb90`、expand `0x00a60120`。导出的 C 仅作为分析辅助，仍以指令和真实消息为依据。

**业务帧加密与首条任务**

目标的协议 210 业务帧采用 AES-128-CBC + 截断 HMAC-SHA1，不能照搬 AlphaBuild 老大厅的 3DES 外层：

```text
[u32 LE length][AB][85][u32 LE sequence][16-byte IV][AES ciphertext][8-byte MAC]
plaintext = [u32 LE body_length][u8 message_kind][body][alignment padding]
```

MAC 覆盖从 length 前缀开始到 ciphertext 末尾的全部字节。认证后序号从 1 开始。72 字节派生材料分为客户端 MAC key 20、服务器 MAC key 20、客户端 AES key 16、服务器 AES key 16；接收路径 VA `0x00a614c0` 也确认了这一结构及序号检查。

状态注册71/4的本版构造器为VA `0x00a48800`，参数为两个16字节blob、build UInt32、两个可选blob和有界字符串。业务帧由已认证方向key验证MAC后解析。

观察模式只回复已核对的注册任务；完整本地服务模式按各业务处理器返回结果或明确失败，不统一空成功。

观察／诊断会话有明确预算，持续游玩使用独立运行策略，见[本地游玩](LOCAL_PLAY.md)。这些预算不声明为官方协议常量。

## 地址发现与业务接入

STUN 支持 type30→31、20→21、14→15，使用观测到的客户端 IPv4／源端口；主／备用端口为 3074／3078。采集额度与协议响应分离，持续游玩不会因记录已满而停止应答。不同 IP 的 Open NAT 和公网 UDP 直连另行验收。

启用 `--service-data-dir` 后按[任务 FIFO 和存储格式](SERVICE_INITIALIZATION.md)处理业务，不支持的任务明确失败。观察模式只用于采集。完整运行请使用[本地游玩](LOCAL_PLAY.md)或[启动器契约](NATIVE_LAUNCHER.md)，不以 AUTH／握手成功替代业务或玩法验收。

回归涵盖独立票据及 AES/HMAC 向量、坏长度／MAC／序号、票据归属与过期、TLS `100 Continue`、未知任务和临时证书恢复。原生票据接受与大厅交互已有实测；原始会话材料仅保存在忽略目录。
