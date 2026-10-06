# Rust AUTH 与协议 210 大厅通道

日期：2026-10-06。目标样本 SHA-256：`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。

后续更新：已加入可选时间/文件服务与任务 FIFO 分发；最新真实实验定位到必需公共配置失败。请从 [SERVICE_INITIALIZATION.md](SERVICE_INITIALIZATION.md) 接续。下文保留建立认证/传输链时的实验与当时限制；业务响应“未实现”的描述对应当时的观察模式。

当前真实客户端已接受 Rust 生成的本地测试票据，随后与同一个 Rust 进程完成大厅协议 210 握手。补齐 UDP 地址发现后，已取得存储、商城、时间等初始化请求。认证与大厅初始化的协议入口已验证；大厅界面、档案、背包和持久化仍未完成，第一阶段整体尚未通过。

**本轮闭环证据**

| GameHub 实验目录（research-output/gamehub 下） | 结果 |
| --- | --- |
| client-1791261761920 | Rust TLS 前 EOF；文件日志显示认证线程在 `CODOL/IIPS/cacert.pem` 查找证书，返回文件不存在 |
| client-1791261916917 | OpenSSL 对照实验成功；读取位置为 `CODOL/cacert.pem`，收到真实 AUTH body |
| client-1791262214867 | 临时补齐两个 CA 位置后，Rust TLS 成功，AUTH 返回动态票据；客户端连接 A，发出 28 字节大厅 preface |
| client-1791263024812 | Rust 发出 server hello，接收 153 字节 client auth，验证其证明，再发送 server auth；收到 114 字节加密业务帧 |
| client-1791263659786 | Rust 校验该业务帧 MAC 并解密，得到 kind 0x86 / service 71 / task 4；按已恢复状态任务格式发出回执 |
| client-1791264500229 | 延长大厅空闲窗口；后续诊断任务报告 Address Fail / DW_LIVE_DISCONNECTED，客户端退出 255 |
| client-1791265072825 | 引入 UDP 观测/应答；捕获 IP discovery `1e 03 00`，确认目标使用版本 3；最初只处理 v2，尚未解除阻塞 |
| client-1791265312422 | 增加经目标代码核对的 v3 应答；客户端继续请求 service 10/12、10/21、80/70、12/6 等，共记录到 32 帧研究上限 |

这些成功实验没有使用 Kernel33、没有启动 AlphaBuild 服务，也没有改主程序。TLS、AUTH 票据、大厅握手和业务加解密均在我们的 Rust 进程内执行；TLS 使用 OpenSSL 库绑定，未再通过 s_server 子进程承接。

**握手前断连的修复**

失败日志中的同一主游戏进程先连接端口 39002，然后 `CreateFileW("cacert.pem")` 解析到 IIPS 子目录，返回 c0000034，随后关闭 socket；ClientHello 尚未发送。成功对照读取了客户端根目录。问题已经定位为相对 CA 文件路径，具体切换工作目录的调用者还未逐函数确认。

实验工具现在仅在研究副本的 `cacert.pem` 和 `IIPS/cacert.pem` 临时提供含实验 CA 的相同 bundle。对原本不存在的文件在退出时删除，对原本存在的文件恢复原内容；备份留在本次实验目录。新增两项 Python 测试覆盖这两种恢复情况。这是研究环境适配，原始客户端目录和系统信任库不变。

**AUTH 票据实现**

`crates/protocol/src/ticket.rs` 实现 128 字节小端布局、Tiger/192（Tiger1）、Tiger(LE32(seed)) IV 和无额外 padding 的 3DES-CBC；与 AlphaBuild 的独立 Python/PyCryptodome 生成结果逐字节一致。参考来源及合成参数保存在 `fixtures/protocol/auth-ticket-alpha-vector.json`。

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

首个真实帧的 MAC 与客户端方向的 key 匹配，解密得到 kind=0x86、service=71、带 UChar 标签的 task=4。其构造函数定位到 VA `0x00a48800`，参数与实际数据一致：两个 16 字节 blob、build UInt32、两个可选 blob、有界字符串。当前客户端报告 build=0。

Rust 仅对这个已核对的状态任务生成 msgKind=1 的任务回执（transaction ID、code=0、task=4、result count=0）。补齐地址发现后，已出现后续 service 10/80 请求；其业务响应尚未实现，因此不能宣称数据初始化完成。其余服务只记录，不统一返回空成功。

首批实验大厅空闲窗口为 10 秒。为排除窗口过短影响后续加载，代码已改为握手阶段 10 秒、认证后空闲 60 秒、单连接总时长 120 秒。该限制属于研究采集预算，不是原协议的超时常量。

**地址发现是进入下一阶段的必需环节**

在没有 UDP 应答时，客户端通过已认证大厅通道发送 71/3、71/7、71/9 等诊断/结束消息，其中出现 `Address Fail (result=-3)` 与 `SF_UI_DW_LIVE_DISCONNECTED`。这不是 AUTH 票据拒绝。

实际 UDP 请求为 IP discovery `1e 03 00`、NAT discovery `14 02 00 <flag>`，以及成功后每约 15 秒的 29 字节 type 14 keepalive。IP discovery 的版本为 3，与旧参考常见的 2 不同。目标 VA `0x00a7eb90` 接受 v2/v3，并在 v3 读取第二个地址；VA `0x00a7ead0` 对其地址/端口执行 XOR cookie 还原。对应的 IP/port reader 是 `0x00a39910` / `0x00a37e20`。

Rust 现提供 type 30→31、20→21、14→15 应答，报告实际观测到的客户端 IPv4/源端口。主端点为 127.0.0.1:3074，备用端点为 127.0.0.1:3078；变更端口请求从备用 socket 回复。本机不能直接 bind 127.0.0.2，本轮没有更改系统网络别名，因此不宣称已验证“不同 IP 的 Open NAT”行为。当前同机应答已足以让客户端进入后续大厅数据请求。

已捕获的下一批请求：

| 服务/任务 | 实际观察 |
| --- | --- |
| 10/12 | 请求 `the.json`、`FileBriefMessage` |
| 10/21 | 请求 `mp_playlists_shipRetail_release.info`、`broadcast.info`、`tlogconfig.info`、`sku_tags.info`、`activities_v1.info`、`configs_s.info`、`market-down.info`、`whitelist_tester.info` |
| 10/10 | 携带 `the.json` 和 blacklist/Recentmetlist JSON 的写入形状；完整参数语义待核对 |
| 80/70 | 商城/玩家数据初始化候选；确切方法名及响应布局待核对，不能直接替换成旧版 80/30 |
| 12/6 | 时间查询候选，与旧参考任务一致，响应尚未接入 |
| 6/5、8/1、8/2、9/14、3/53、29/10 | 已捕获并记录，业务语义/应答待确认 |

最新实验触及 32 帧采集预算并在缺少业务响应的情况下退出 255，不作为完整大厅运行或稳定性通过的证据。

**代码入口与重现**

| 文件 | 职责 |
| --- | --- |
| `crates/protocol/src/ticket.rs` | 票据编码、摘要和 3DES |
| `crates/protocol/src/lobby.rs` | 210 preface、位缓冲区、HKDF、AES/HMAC、71/4 结构 |
| `crates/server/src/auth_server.rs` | TLS/HTTP、动态票据、本地身份 |
| `crates/server/src/session.rs` | 票据到用户/会话密钥的绑定 |
| `crates/server/src/lobby_server.rs` | 大厅握手、验证、有限状态回执和观测 |
| `crates/protocol/src/stun.rs` / `crates/server/src/stun_server.rs` | 目标版本 UDP 地址发现、NAT/keepalive 及抓取 |

```sh
sh tools/check.sh
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --seconds 100 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun --trust-lab-cert
```

实验 CA 必须仍在有效期内；过期时按 GAMEHUB_DEBUG.md 生成新的短期证书。工具在项目的独立 prefix 与客户端副本中运行，结束时关闭自己的进程并恢复 CA。

也可以只启动后端，由操作者另行管理客户端副本：

```sh
sh tools/cargo.sh run --locked -p codol-server -- serve-auth \
  --cert .tools/tls-lab/server.pem --key .tools/tls-lab/key.pem \
  --local-title-id 1002 --lobby-bind 127.0.0.1:39001 \
  --stun-bind 127.0.0.1:3074 --stun-alt 127.0.0.1:3078
```

所有入口仍限制回环地址。省略 local-title-id 时保持记录/501 模式；reply-file 是独立的实验响应模式，与动态票据互斥。

输出在 `auth-captures/auth-*/`：每次 AUTH 的请求、响应、私有会话记录；`lobby/<connection>/` 保存 hello/auth、握手结果、加密帧、MAC 已验证的明文和任务摘要。真实票据/业务明文留在忽略目录。可提交的帧、KDF 与加密向量均使用合成参数；单独的真实 preface 只包含协议版本、缓冲区大小和公开随机 nonce。

**验证与下一步**

当前回归覆盖独立票据向量、Python SHA1/HMAC/AES 向量、截断/长度/MAC/序号错误、无符号 UID、TLS 100 Continue、两次签发密钥不同、未签发票据拒绝、注册回执及未知任务不回假成功、临时证书恢复。完整检查入口为 tools/check.sh。

下一步从已捕获的 10/12、10/21、80/70 请求出发，核对对应构造器/反序列化器，补齐任务关联与响应，逐步恢复公共文件、时间、档案、playerData 和 inventory。build=0 和缺少 buildconfig.ini 仍是待查线索，尚未证明会阻止这些请求。不要直接发送旧版 220/11 档案种子；服务 80 的新任务号已显示存在版本差异。
