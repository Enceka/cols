# CODOL-AlphaBuild 实现审阅与 3.3.3.3 适配结论

日期：2026-10-06。参考目录：`/Volumes/Projects/codol/CODOL-AlphaBuild`。本轮审阅源码、配置和文档，并与目标客户端静态代码及已有抓包对照；未启动参考包的客户端、大厅或原生服务，未修改其代码、玩家档案或数据库。

结论：这份资料能显著缩短登录、背包和大厅的重建路径。最先采用认证票据、Tiger/3DES 和带类型标签的大厅编码知识；目标仍为 3.3.3.3，保留已实测的 HTTPS JSON 认证入口。参考包针对 1.1.1.13，字段偏移、任务记录和资源数据需要逐项对照。

实施更新：参考票据已用 Rust 重建并获真实客户端接受；目标大厅新增协议 210，已另行恢复其 HKDF-SHA1/AES/HMAC，不能直接使用本参考的旧大厅 3DES 帧。最新验证见 [AUTH_LOBBY_PROGRESS.md](AUTH_LOBBY_PROGRESS.md)。以下保留审阅时的证据和判断。

**2026-10-07 联机资料复核**

用户提示参考更新，已重新审阅 dedicated 的 `CODOL_ASSOCIATION.md`、handshake/ecdh/crypto/association/network/connect 源码和测试，以及 lobby 的 admission/reconnect 文档。目录无 `.git` 元数据，只能记录当前快照，不能确认精确增量；7 个重点文件指纹在忽略的 `research-output/alpha-network-review-20261007.json`。另核对新增 `CODOL_AlphaBuild_LAN 服务器端/CODOL_Server`：上述 association 文档、crypto/association/connect 三份源码和 admission/reconnect 两份文档共 6 项与主参考目录逐字节一致；比对记录为 `alpha-lan-comparison-20261007.json`，不能据此推断两个目录全部相同。

采用的协议研究线索包括 P-224 公钥 DER、cookie 报文结构、实时会话 key 与 admission 隔离、重新连接清理。3.3.3.3 静态与真实包验证得出游戏 DATA 为 SHA1 KDF、AES128-CTR、小端计数、SHA1(seq32+tag16) IV、HMAC 覆盖长度字段，不能复用旧版 Tiger/AES192。参考固定私钥兼容实验不适用于本轮目标：随机私钥已实测成功。Rust 独立重建，未移入原生 C/C++ 源码；详细地址、实验与尚缺功能见 [游戏通道进展](GAME_TRANSPORT_PROGRESS.md)。

**参考包实际包含什么**

| 部分 | 实际实现 | 对当前任务的价值 |
| --- | --- | --- |
| `CODOL_Server/lobby` | Python Demonware/auth、存储、房间、匹配、商城/背包、档案、SQLite、对战交接 | 第一阶段的主要参考 |
| `CODOL_Server/patcher` | Kernel33 导入代理、嵌入的旧版解包代码、特定地址补丁 | 解释参考包的客户端启动前提；补丁地址仅针对该构建 |
| `CODOL_Server/dedicated` | CoD4x/IW4OL 派生的 C/C++ 与保留汇编，引入 CoDOL 网络、快照、资源、脚本与玩法模块 | 第二阶段对战的行为和结构参考，不是现成的 Rust 对战实现 |
| `CODOL_Client` | 已改动 Kernel32 导入的旧版客户端、备份、资源和运行数据 | 本轮只做样本身份及配置核验 |

版本依据：lobby README、patcher 的 BUILD_TIMESTAMP、dedicated 的 connectResponse 构造均明确指向 1.1.1.13。不能用目录名 AlphaBuild 推断为 3.3.3.3。[大厅说明](../../CODOL-AlphaBuild/CODOL_Server/lobby/README.md)、[补丁常量](../../CODOL-AlphaBuild/CODOL_Server/patcher/kernel33.c)、[对战连接回复](../../CODOL-AlphaBuild/CODOL_Server/dedicated/src/codol/net/codol_connect.c)

| 样本 | 字节数 | SHA-256 |
| --- | ---: | --- |
| 目标 `CODOL/codoMP_client_shipRetail.exe` | 10473472 | `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b` |
| 参考 `CODOL_Client/codoMP_client_shipRetail.exe` | 20918752 | `904436480938632a856b39dea73fb94c9682b5a0ebf7fa619ecd5dd5475ed3b6` |
| 参考 `CODOL_Client/codoMP_client_shipRetail.exe.bak` | 20918752 | `a6c213c5664e8ca4bad8d3fcaf58d2ac4c4bb110c4d13bee69770bc6cee30bd9` |

**认证：可优先复用票据核心，外层消息格式有差异**

参考 `demonware/dw_auth.py` 的关键函数是 `build_ticket`（第 100 行）、`build_auth_reply`（第 185 行）和 `build_lobby_reply`（第 274 行）。[源码](../../CODOL-AlphaBuild/CODOL_Server/lobby/demonware/dw_auth.py)

该实现的 AUTH 外层是 TCP 长度前缀、加密类型和位打包消息，任务请求 10、响应 11、成功码 700。我们的 3.3.3.3 已实际发出 HTTPS `POST /auth/` 和 JSON，响应读取代码也读取 JSON 的 auth_task/code/iv_seed/client_ticket/server_ticket。因此可以复用核心构造规则，但必须用 3.3.3.3 已确认的传输与 JSON 表达承载它们。

参考中的 128 字节票据按小端打包为：

| 字节偏移 | 长度 | 参考含义 |
| --- | ---: | --- |
| 0 | 4 | magic，默认 0xEFBDADDE |
| 4 | 1 | type |
| 5 | 4 | title_id |
| 9 | 4 | issued_at |
| 13 | 4 | expires_at |
| 17 | 8 | license_id |
| 25 | 8 | user_id |
| 33 | 64 | username |
| 97 | 24 | session_key |
| 121 | 3 | usingHashMagicNumber，参考值 09 04 00 |
| 124 | 4 | Tiger(前 121 字节) 的前 4 字节 |

参考票据加密采用 Tiger("simplePassword") 的 24 字节结果作为 3DES key，IV 为 Tiger(LE32(iv_seed)) 的前 8 字节，CBC 不添加额外 PKCS#7 块，因为票据已经是 128 字节。

本轮在 **3.3.3.3 自己的二进制** 中核对了以下链路，避免只依赖参考注释：

- VA `0x006d2f70` 的 PC 登录分支把 VA `0x00c7e34c` 的 `simplePassword` 传给 `0x00a58870`；该方法将结果写入对象 `+0x3180` 并设置 auth_task=10。
- `0x00a58870` 调用 `0x00a52490`，后者按字符串长度调用 24 字节摘要方法 `0x00a3ac40`，其算法查找字符串 VA `0x00c07450` 为 `tiger`。
- 解密路径 `0x00b2c3f0` 调用的密码对象构造函数为 `0x00b2dcf0`；其注册描述符 `0x00c43340` 的算法名为 `3des`，密钥长度检查为 24 字节，块长为 8。
- `0x00b2c4b0` 使用同一个 24 字节摘要对象处理 4 字节 IV seed。这与参考 IV 推导方式吻合。
- `0x00a678c0` 从 128 字节输入连续读取 `4,1,4,4,4,8,8,64,24,3,4` 字节，与参考票据布局完全对齐；magic 常量 VA `0x00c07908` 的实际字节为 `de ad bd ef`。

置信度：票据布局、默认密码调用链和 Tiger/3DES 算法已有本地静态佐证；hashMagic 的具体值、校验覆盖范围、加密后的票据是否被 3.3.3.3 接受，仍需用真实客户端验证。不能把格式一致直接计为认证成功。

参考 `build_auth_reply` 将同一个加密票据写入两个 blob；3.3.3.3 的两个 JSON 票据字段如何分别处理，仍以该版本的读取及校验为准。

参考 AUTH 默认固定 user_id=1、固定 username 和 session_key=0..23。它后续再从存储请求里识别玩家 UID，另有 admission 模块。Rust 实现应把测试身份、票据和后续连接状态绑定，不能把这些固定测试值当成完整的多玩家身份模型。

**大厅：编码与服务划分可作重点参考**

`demonware/dw_wire.py` 分离了帧加解密和带类型标签的值编码。[源码](../../CODOL-AlphaBuild/CODOL_Server/lobby/demonware/dw_wire.py)

参考加密帧为 `[u32 length][u8 encryption_type][u32 message_count][ciphertext]`，用 Tiger(LE32(message_count)) 推导 IV，再做 3DES-CBC。其服务端到客户端明文以 0xDEADBEEF 开头；客户端到服务端的前四字节有不同用途，不能把两个方向的验证逻辑合并。

Blob 的长度本身也带 UInt32 类型标签，即 `[tag19][tag8][u32 length][bytes]`。这个细节可直接变成 Rust 编解码的候选测试向量，但 3.3.3.3 的对应接收/发送路径仍要复核。

参考的服务候选清单：10 存储、11 房间、12 时间等工具、21 匹配、80 商城/背包、4 统计、102 聊天。它们帮助定位 3.3.3.3 的构造函数，暂不把任务 ID 全部写死成已验证协议。

`dw-capture.py` 第 1007–1019 行仍让未处理任务退回 `err=0, results=[]`。这可以让流程继续，但不能证明对应功能已恢复；新实现继续记录未知任务及客户端解析结果。[实际分发](../../CODOL-AlphaBuild/CODOL_Server/lobby/dw-capture.py)

**背包：最重要的是三条数据链一起恢复**

第一条是物品目录及持有记录。`dw_marketplace.py::inventory_record` 当前返回 **八个**带标签字段：UInt64、UInt32、UInt32、UInt32、UInt32、Int64、UInt16、UInt32。字段含实例、物品、数量、武器 XP 与期限等；文件上方保留了已被修正的七字段旧描述，应该以第 486–489 行的实际 writer 和其更正记录为准。[背包记录](../../CODOL-AlphaBuild/CODOL_Server/lobby/demonware/dw_marketplace.py)

第二条是大厅重建时的数据。最新 `inventory_delivery.md` 首段明确修正：旧问题来自 `80/30 getPlayerData` 返回零记录导致持有表被清空。当前 `playerdata_record`（第 1517 行）组合钱包、entitlements 和解锁标志；`build_playerdata_reply`（第 1564 行）确实返回一个真实记录。仅恢复 `80/10 getInventory` 不足以保证重新进入大厅后背包仍正常。[修正记录](../../CODOL-AlphaBuild/CODOL_Server/lobby/docs/inventory_delivery.md)

第三条是装备与档案存储。`dw_storage.py` 处理 mpdata/mpdata_s，`store` 下的 codec、profile 和 SQLite 模块负责解码、渲染与持久化。参考 mpdata 的定义版本是 220，mpdata_s 为 11 / checksum 0x096B8F57；这些是 **1.1.1.13 的数据**，需要从 3.3.3.3 的 IIPS/资源定义重新确认，不能直接把旧 seed、物品 ID 或偏移送给新客户端。[存储服务](../../CODOL-AlphaBuild/CODOL_Server/lobby/demonware/dw_storage.py)、[档案 codec](../../CODOL-AlphaBuild/CODOL_Server/lobby/store/mpdata_tool.py)

参考 SQLite 把 UID 存为 TEXT，与我们已观测到的无符号 64 位用户 ID 相符；账户、文件、档案、钱包、背包、购买流水也分表处理。这些数据组织原则值得沿用，具体字段以新版本需求为准。[数据库 schema](../../CODOL-AlphaBuild/CODOL_Server/lobby/store/lobby_db_schema.py)

**客户端补丁与对战服务的边界**

`patcher/kernel33.c` 的时间戳 0x5314DEF0、入口 VA 0x00ADFAFA、解包区域和渲染补丁都针对旧构建。参考客户端实际已将 Kernel32 导入名改为 Kernel33，不能据“同名 EXE”套用到目标样本。

对战目录包含原生网络、快照、移动、武器、GSC、资源、碰撞等实现及大量研究资料，后续应按模块映射到 3.3.3.3。其 README/PORT 文档仍写着 incomplete；更晚的 IMPLEMENTATION_PASS 又记录了部分构建、探针和运行进展。两者存在历史状态差异，未实际运行前不将该包认定为完整可玩的成品，也不忽略后期代码已经完成的工作。[原生说明](../../CODOL-AlphaBuild/CODOL_Server/dedicated/README.md)、[后期记录](../../CODOL-AlphaBuild/CODOL_Server/dedicated/docs/CODOL_IMPLEMENTATION_PASS.json)

原生目录携带 CoD4x 来源和 AGPL3 许可文件；后续若移入代码，需要记录对应来源和许可，协议事实与直接移植的源码分开记录。本轮没有移入参考源码。

**后续实施顺序**

1. 依据共享布局在 Rust 构造 128 字节票据，先用 Tiger 和 3DES 已知向量校验，再放入现有 HTTPS JSON 响应；优先验证客户端是否从 AUTH 转向端点 A。
2. 根据端点 A 的真实首包恢复 LSG 连接与加密帧，将参考 dw_wire 的公式作为对照，不沿用固定全局会话状态。
3. 逐条恢复客户端真正请求的 storage、时间、playerData、inventory；同步定位 3.3.3.3 的物品目录和两类档案定义。
4. 用“进入大厅→查看/修改装备→重登→状态保留”验收第一阶段；后续才安排原生对战资料到 Rust 的模块适配。

**本轮验证与工作状态**

- 已核对上述关键源码和三个客户端文件哈希；未运行参考包服务。
- 只执行了已阅读的纯计算 Tiger 自检，使用 `python3 -B` 禁止写 pycache；两个内置向量均通过。它验证参考 Tiger 的给定结果，不代表 3.3.3.3 已接受票据。
- 未运行参考包完整自测：其中涉及本地状态与平台依赖，不把 README 的“全部通过”当成本轮测试结果。
- 上一轮暂停前，我们自己的仓库已新增 Rust `serve-auth` TLS/HTTP 入口和三项传输测试，测试通过；首次真实 GameHub/Rust 实验在 ClientHello 前 EOF，尚未实测成功。此前成功的真实 TLS/AUTH 证据来自 OpenSSL 探针。后续应复测这个已写好的 Rust 入口，避免重复搭建。
- 本轮新增此审阅文档及接续索引，没有继续启动 GameHub、Ghidra 或服务器实验。
