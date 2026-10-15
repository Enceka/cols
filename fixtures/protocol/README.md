# 协议样本

`room-loadout-synthetic.json` 按本版 `0x6df130/0x6df360` 的649位 type18 格式构造角色、武器及装备物品向量，使用独立字节移位编码；不包含玩家身份或账号数据。消费链与原生验证见 [房间 Bot 角色展示](../../docs/ROOM_BOT_AVATARS.md)。

`progression-3.3.3.3.json` 是只读本版资源表提取的三套数字经验门槛，人物／主武器／副武器各70／31／20行，rank表版本4。来源指纹、提取命令及等级提示待验收项见 [等级接入准备](../../docs/PROGRESSION_PREPARATION.md)。没有真实玩家进度；不使用Alpha旧曲线。

`clienthello-3.3.3.3.bin` 是真实客户端连接研究端点 B 时发出的 70 字节 TLS ClientHello。仅包含公开握手信息，不含认证请求、账号或会话票据；来源及哈希见同名 JSON。

`auth-task10-sanitized.json` 根据真实 TLS 明文请求的结构制作：保留字段名、字符串类型、双层 JSON 表达、已观测的 title_id=1002，以及在主程序 VA 0x00c07e5c 中确认的固定 identity。iv_seed 与 user_id 分别替换为合成值 1 和 2。这不是逐字节原始抓包；原始请求只保存在被忽略的 research-output 目录。

这两类早期样本只验证请求解析，不能独自证明登录成功。后续真实票据与大厅通道的验证见新的进度文档。

后续新增：

- `auth-ticket-alpha-vector.json`：AlphaBuild 纯计算函数生成的合成票据向量，记录源文件哈希；用于独立对照 Rust 的布局、Tiger、IV 与 3DES。
- `lobby-clienthello-3.3.3.3.bin/json`：真实的 28 字节协议 210 preface；仅协议常量、接收上限和公开 nonce，没有票据或用户标识。
- `lobby210-synthetic.json`：独立 Python SHA1/HMAC/AES 与字面位写入器生成的合成握手和业务帧向量，所有会话材料都是测试值。
- `publisher3333.json`：从目标样本独立恢复资源密钥后，用 PyCryptodome 生成的 AES-192-CTR 向量；明文为合成字节或本地研究配置，未包含内嵌 RSA 私钥。
- `public-profile-synthetic.bin/json`：Python zlib/CRC32 生成的合成 32 KiB 档案及 125 个 Int32 投影字段；仅验证格式和完整性，不是真实玩家档案，也不能作为游戏档案种子。
- `room-roster-synthetic.bin`：来自3.3.3.3的type14完整房间成员表及定向manifold结构；人类UID替换为42，其他成员元数据清零，只保留实验Bot合成ID、阵营和准备位。验证普通广播内发给模拟Bot的名单不能被遗漏，不包含真实账号或会话资料。
- `room-browser-synthetic.json`：由本版原生消费者确认的 query4 四条件、普通／分页请求、11/11 双 UInt64 入房参数及保留真实房主结构的列表记录；独立 Python tagged 编码生成，身份和密钥为合成值。分页使用末条 session ID，列表项入房使用另一个 channel ID；不是大厅展示或真人入房的实机验收。

真实票据接受与大厅握手证据另见 docs/AUTH_LOBBY_PROGRESS.md；不将真实会话密钥、票据或业务明文加入这些 fixtures。

`damage-feedback-resources-3.3.3.3.json`记录三图各自NCS捕获SHA/CRC、反馈材质槽和当前声音消费者。仅含资源身份和布局元数据；运行配置由准备器复算，原始采集仍在忽略目录。

`death-waypoint-hud-3.3.3.3.json`固定本版HUD两组读取位置、waypoint绘制器、坐标codec/零中心、旗标与材质采集身份。发送顺序为archival后current；死亡标识属于current，命中反馈属于archival。

`voice-3.3.3.3.json`记录本版声音选择器、播放／停止、角色前缀消费者及阿富汗CRC核对的资源例子。完整声音目录由`tools/prepare_voice.py`按各图采集生成，不能把例子编号或参考脚本时机当作已通过实机听感验收。

`killstreaks-3.3.3.3.json`由`tools/prepare_killstreaks.py`核对本版原程序、只读资源表／schema及原生HUD采集生成，仅含必要奖励身份、成本和资源标识、60项背包四槽偏移、PS字段／消费者摘要及来源哈希。完整29×33资源表与HUD表达式保留在忽略目录；44个商城条目中23项精确匹配，21项Pro未匹配，没有升级版别名映射。`evidence_only=true`／`gameplay_verified=false`明确不代表发奖、调用或效果验收；接入方案见`docs/KILLSTREAK_PREPARATION.md`。
# 团队模式

`team-modes-3.3.3.3.json` 为本版模式表及中文资源的只读摘录：`war`／别名`tdm`是战术，`cha`是冲锋。NUMBER／token／alias／name列为3／4／5／6，中文原字节及GB18030解码、资源loaded标志分别保留；缓存名称不代表该模式已进入或资源已准备。

`charge-map-registry-3.3.3.3.json` 保存本版 `coMapList.csv` 的17条冲锋地图身份、原生ID、人数与开启标志，`resource_preparation_claimed=false`。实际开放准备和原生证据见 `docs/CHARGE_MODE.md`，不以表中身份或战术捕获替代冲锋资源验收。
