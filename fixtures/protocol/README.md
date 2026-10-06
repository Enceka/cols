# 协议样本

`clienthello-3.3.3.3.bin` 是真实客户端连接研究端点 B 时发出的 70 字节 TLS ClientHello。仅包含公开握手信息，不含认证请求、账号或会话票据；来源及哈希见同名 JSON。

`auth-task10-sanitized.json` 根据真实 TLS 明文请求的结构制作：保留字段名、字符串类型、双层 JSON 表达、已观测的 title_id=1002，以及在主程序 VA 0x00c07e5c 中确认的固定 identity。iv_seed 与 user_id 分别替换为合成值 1 和 2。这不是逐字节原始抓包；原始请求只保存在被忽略的 research-output 目录。

这两类早期样本只验证请求解析，不能独自证明登录成功。后续真实票据与大厅通道的验证见新的进度文档。

后续新增：

- `auth-ticket-alpha-vector.json`：AlphaBuild 纯计算函数生成的合成票据向量，记录源文件哈希；用于独立对照 Rust 的布局、Tiger、IV 与 3DES。
- `lobby-clienthello-3.3.3.3.bin/json`：真实的 28 字节协议 210 preface；仅协议常量、接收上限和公开 nonce，没有票据或用户标识。
- `lobby210-synthetic.json`：独立 Python SHA1/HMAC/AES 与字面位写入器生成的合成握手和业务帧向量，所有会话材料都是测试值。
- `publisher3333.json`：从目标样本独立恢复资源密钥后，用 PyCryptodome 生成的 AES-192-CTR 向量；明文为合成字节或本地研究配置，未包含内嵌 RSA 私钥。
- `public-profile-synthetic.bin/json`：Python zlib/CRC32 生成的合成 32 KiB 档案及 125 个 Int32 投影字段；仅验证格式和完整性，不是真实玩家档案，也不能作为游戏档案种子。

真实票据接受与大厅握手证据另见 docs/AUTH_LOBBY_PROGRESS.md；不将真实会话密钥、票据或业务明文加入这些 fixtures。
