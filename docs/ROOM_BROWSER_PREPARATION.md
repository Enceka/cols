# 原生房间列表与多人入房路由

目标为未修改的 3.3.3.3 客户端与纯 Rust 服务端。客户端主程序 SHA-256 为
`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。
下面的地址均为该 PE32 样本的 VA，不能套用 AlphaBuild 的 C1 地址或结构。

当前已接入基础 MP 房间查询、稳定分页、认证成员加入／退房、名单获取及跨连接
消息投递。创建和地图状态继续使用原生描述，单人 Bot 房间仍按当前完整人数和容量
处理。原生界面的列表显示、两个真实游戏客户端入房仍需单独验收；多人共享对局尚
未实现，多个真人成员的房间明确拒绝游戏服分配，不把独立玩家世界当作共享对局。

## 原生入口与协议分工

前端导出的 `ListRoomTask.onStart` 调用 `CFunctionContainer.ListRoom(type, mode)`。
`MatchController.listRoomNow` 维护房间列表缓存，强制刷新取消旧任务；
`MatchSelectPvpView` 在收到 `finished` 后停止追加，否则继续取页。
`MatchService.fakeRoomList` 返回 null，实际 Scaleform 路径仍依赖原生返回。

| 行为 | 本版原生调用链 | 服务端任务 |
| --- | --- | --- |
| 列出房间 | ListRoom 绑定 `0x6acf70` → `0x6e3200` → `0x6e3310` | `21/5 query=4` 或 `21/13 query=4` |
| 普通查询 | `0x6d4900` → SDK `0xa56f50` | `21/5` |
| 分页查询 | `0x6d4810` → SDK `0xa570d0` | `21/13` |
| 点击列表项加入 | JoinRoom 绑定 `0x6ad150` → `0x6e3930` → `0x6e3490` → `0x6c9fe0` → `0xa4db00` → `0xa4ca40` | `11/11` |
| 成员名单 | 入房回调 `0x6e8720`／`0x6e8550` → `0x6ca0d0` → SDK `0xa4d140` | `11/15` |
| 自建房登记／更新／删除 | 当前 Rooms 已处理 | `21/1`、`21/12`、`21/3` |
| 频道广播／定向发送／离开 | SDK 频道与原生房间协议两层共同工作 | `11/4`、`11/5`、`11/2` |

`21/5 query=3` 是快速匹配找同伴，`query=1` 是准备后的游戏服分配，均不等于大厅
浏览。`room::Search` 处理 1／3；`room_browser::Search` 的 query 4 与 `21/13` 在业务路由中先分流，不能进入 allocate 分支。

原生页面的 MP 房间类型为 0，ServerChannel.MP_ROOM 为 0；MP_PLAYLIST 为 1。
这些 UI 分类与 SDK 的 UInt64 房间频道 ID 是不同概念。

## 查询参数与分页

字段使用现有 Demonware tagged 编码：UInt32=8、Int32=7、Bool=1、UInt64=10、
Blob=19（后接 tagged UInt32 长度），任务参数最后是 NoType=0。

```text
21/5:
  UInt32 query=4
  UInt32 offset
  UInt32 maximum
  4 × Int32 criteria
  NoType

21/13:
  UInt32 query=4
  Bool initial
  Blob[8] cursor
  UInt32 pageSize
  4 × Int32 criteria
  NoType

11/11:
  UInt64 channelId
  UInt64 credential
  NoType
```

条件序列的确定依据是 `0x702520` 的四次 Int32 写入；其基类 `0x7d0550` 不写字段。
不能采用 AlphaBuild 的六个条件。`0x6e3200` 填入的四个值依次来自：

1. `(dvar[0x0315fbec]+0xc | 0xd6f01f90) XOR optional 0x800000`：与当前创建／分配
   描述中的 build 对照；已有正常样本为 `-688906341`。
2. 常量 0。字段的完整过滤语义尚未恢复。
3. 在 `0x037489d8` 所指开关启用时取全局 `0x07838108`，否则为 2。
   不能仅凭位置把它当作玩家等级或最低 DLC 等级。
4. 房间控制器 getter `0x6e5710` 返回的 `+0xf848` 值。浏览入口检查该值为 0
   后才发请求；其它分类／模式入口需单独核对。

分页对象由 `0xa55840` 初始化，`+4` 为状态、`+8` 为 8 字节游标、`+0x10` 为页大小。
`0xa570d0` 在状态 0 时写 initial=true，状态 2 时禁止再次发请求；页大小是运行时
设置值，不能将旧版样本的 20 当作本版固定协议常量。当前 codec 的 1..50 是本地
预算，并非原生所有部署的上限。

`0xa45190` 先解码普通任务结果数组，再将结果指针和实际返回数量交给分页对象。
`0xa558b0` 从最后一条记录 `+0x10` 复制 **session ID** 作为下一页游标：

- 返回数量小于 pageSize 时，状态变为 2，查询完成。
- 返回满页时，状态变为 1，允许继续查询；恰好满页的末页需要下一次空页结束。
- 返回没有额外的分页包装记录或独立游标字段；普通与分页使用同一种匹配记录。
- totalNumResults 不是该分页消费者判断结束的依据。

服务端分页按认证会话与完整条件建立有界查询快照，保持记录顺序和 session ID
稳定，按上一条 session ID 继续。首次查询忽略所携带游标的内容；续页验证游标属于
该查询。快照有效期不超过60秒，最多128个，页大小变化要求重开查询。房间离开、删除或到期后不得从旧快照重新发布其有效准入信息。

## 列表回包与身份

结果数组元素 vtable 为 `0xc78450`，基类解码 `0xa55700`，本版尾部解码
`0x701db0`；stride getter `0x7020f0` 返回 `0x228`。

```text
Blob[37] room common address
Blob[8]  room session ID
UInt32   session kind
UInt32   maximum players
UInt32   current players
String   room name
UInt64   owner UID
String   owner name
3 × Int32
UInt32
3 × Int32
Blob[16] session security key
11 × Int32
UInt64   channel ID
2 × Int32
```

这是创建描述插入 session ID 与 current players 后的形状，结果记录自身没有
NoType 终止字节。未知属性按原始描述保留。

| 消费者确认的字段 | 查询结果对象偏移 | 用途 |
| --- | --- | --- |
| 8 字节 session ID | `+0x10` | 分页游标；不同于 channel ID，也不同于独立游戏服分配 ID |
| maximum players | `+0x120` | 前端 `currentMaxPlayers` |
| current players | `+0x124` | 前端 `currentPlayers` |
| 房间名 | `+0x128` | 列表名称 |
| 房主 UID／名字 | `+0x168`／`+0x170` | 原始房主身份 |
| security key | `+0x1cc` | 会话材料；不能写入公开日志或真实 fixture |
| channel ID | `+0x208` | `0x68c790` 转为十进制字符串放入 RoomObj.id；点击后加入这个频道 |

SDK 字符串上限与原生创建表单／列表过滤不是同一限制。`0x6e2650` 创建表单检查
房间名称 24 字节，`0x6eb140` 会筛除超出原生名称限制的结果。中文名称的端到端
编码仍需实机覆盖；不能通过随意截断 UTF-8 字节来避免客户端过滤。

当前 `Description::allocation_record` 会替换地址与房主为 Local Game Endpoint，
用途是游戏服分配；它不能作为大厅列表记录。`Description::listing_record` 保留
房间描述中的地址、房主、名称和原生属性，只插入房间 session ID 与权威人数。
当前 fixture 中身份、地址与密钥均为合成值，不是可连接的真人房间。

## 从可见到成功加入

正常 `0xa4db00` 调用 `0xa4ca40`，第二个 UInt64 credential 为零；服务端不能将
其当作少一个字段的 UInt64 请求。密码／邀请入房另有前端分支，其 credential 与
校验语义仍需采集，解析支持不等于允许非零凭据入房。

SDK 加入回执只使频道订阅成功。其后 `11/15` 填充 SDK 成员表，客户端再走原生
JoinRequest／JoinResponse、RoomInfo、成员表及准备状态协议。`0x6e8a20` 要求请求
发送者已在 SDK 频道成员表中，缺少成员加入推送会使有效 JoinRequest 被丢弃。

原生房间消息仍复用当前 `room_messages` 中的未对齐 bit stream 与 LE32 批次边界，
不能把它们改成 tagged RPC 字段。已恢复的 type5=JoinResponse、type10=成员更新、
type14=完整成员表、type15=RoomInfo、type16=准备状态可继续使用；真人 JoinRequest
的身份、密码／邀请、party、版本与资源能力由原生接纳路径进一步核对。

SDK event9 的成员＋频道元数据先到，再转发该成员的原生消息。event10 的离开
通知需同步名单；event7 为频道消息，正文依次是发送者 UID、SDK 名称、频道 ID、
Blob、delivery。SDK 名称使用 Base64，`0x6ca190`／`0x6d3830` 解码后交给房间。
当前 `11/15`、event9 与 event7 的名称统一使用Base64。创建描述的房主名是明文；
加入者尚无完整昵称来源时使用认证UID作名称，不能伪造昵称或取用别人的档案。

房主用 `11/5` 定向回复加入者，`11/4` 广播新的房间状态。沿用原生房主的接纳回包
及状态交换，同时由服务端检查频道归属、真实发送者、接收者成员资格与容量；不能
给真人伪造 Bot JoinResponse 作为多人入房完成的证据。

## Rust 接入边界

| 位置 | 当前行为 | 必要接入 |
| --- | --- | --- |
| `protocol/room_browser.rs` | query4 普通／分页请求及 `11/11` 参数解析 | 非默认筛选条件的语义 |
| `protocol/room.rs` | 保留房主／地址／属性的列表记录 | 以 `21/12` 的最新描述显示地图／状态 |
| `server/room_directory.rs` | 查询快照、真实成员、容量、加入确认、退房／过期清理 | 密码／邀请、房主迁移、更多参与者命令 |
| `server/services.rs` | 本连接 push 与指定目标连接的 delivery 分开 | 完整昵称来源 |
| `server/lobby_delivery.rs` | 每个认证连接64条有界邮箱；满队列关闭慢连接并清理成员 | 更多压力与长时间验证 |
| `server/lobby_server.rs` | 单个writer处理RPC与邮箱；读取半包期间保持同一读future | 原生双客户端验收 |
| `server/session.rs` | 新登录撤销旧票据；旧连接不能通过有效MAC恢复已撤销身份 | 更多异常退出／重连组合 |
| `server/game_server.rs` | 现有单人准入／世界 | 每名真人席位与共享权威世界 |

跨连接 writer 同时消费 RPC 和邮箱，RPC 保持 FIFO transaction ID；push 不占
transaction ID，却消耗目标连接自己的 S2C sequence。加入ACK及本连接成员通知先写出，
随后发布跨连接通知。对方不发请求时，邮箱也能唤醒writer。关闭／替换连接和邮箱
过载会清理对应成员；旧连接的退出不会删除替代它的新连接。每5秒清理过期成员。

当前基础查询仅接受 `[build, 0, 2, 0]`，按title与build查找活跃注册房间。其它尚未
恢复语义的条件明确返回错误，不能静默忽略。参与者目前可发送JoinRequest、名单请求
及准备／取消；房主负责JoinResponse、成员表与RoomInfo。参与者不能冒充房主发布
这些权威记录。定向／广播均校验成员身份，游戏DATA与共享对局不在此处转发。

大厅可见、同房准备和同局同步是三个验收边界。现有 UDP Connection 各自持有
CombatWorld，多个 peer 共用地址不能形成共享对局；需要将权威世界、计分、Bot、
tick 与对局生命周期按房间持有，再为每位玩家保留独立输入、加密、ACK 和快照。

当前研究大厅只允许 loopback 监听，游戏 UDP 也筛除非 loopback 来源。局域网部署
还需核对客户端端点、启动器转发与监听策略；同机双客户端通过不能自动证明远端
客户端可连接。

## 原生验收与剩余工作

1. 用两个不同认证身份的原生客户端验证房名、地图、人数、刷新和分页；覆盖空页、
   满页末页、游标失效及房间删除。业务／TCP测试已覆盖这些路由的核心边界，原生
   界面需要独立消费证据。
2. 原生A开房→B搜索→B加入→两端名单一致→房主选图／换队→准备／取消→B退房。
   不把SDK订阅回执或传输测试中的合成JoinResponse状态当作完整原生接纳。
3. 扩展密码／邀请、参与者的其它命令、房主迁移及Bot预留／替换策略。满房拒绝、
   重复加入、待接纳超时、旧票据撤销、异常断线与房主关闭已经接入并有代码验证。
4. 每玩家游戏准入与共享世界仍未接入；后续验证互相可见、移动／伤害／计分同步、
   结算返回及再开局。当前多真人成员房间明确拒绝游戏服分配。

## 证据与复现

房间浏览新增五项协议测试通过，覆盖格式与既有分配查询的隔离；协议模块
Clippy（all-targets、warnings-as-errors）通过。这不是原生大厅可见或真人入房的
验收。

`room_browser_acceptance` 使用临时服务资料、两个合成身份与动态端口的真实
GameAdmissions，直接调用 LocalServices。当前房主登记、普通查询、首屏分页和
第二身份加入全部成功，报告为 `research-output/room-routes-acceptance.json`。

真实TCP传输测试使用两名独立身份完成协议210认证；验证房间查询、加入、双成员
名单、准备／取消、空闲连接成员通知及退房。消息在各自连接上验MAC和S2C序号，
RPC transaction ID保持FIFO；另一个测试在C2S只送达两字节长度前缀时先收到S2C
推送，随后仍完整读取原请求，验证不会因推送取消半包读取。

代码证据为 `lobby_room_transport_tests.rs`、`room_directory_tests.rs`、
`lobby_delivery.rs` 与 `session.rs` 的测试；检查日志保存在 `research-output/room-routes-*.log`。
这些是业务／传输验收，不能替代原生客户端的列表呈现和真正双客户端加入。

正常启动器的现存客户端已通过只读采样确认：前端 connection_state=0／frontend_state=5、6772 项库存，以及
阿富汗房间／局内 state10 和 12 人名单。对应证据在
`research-output/room-browser-live-acceptance/acceptance.json` 及其同目录；实际 Wine prefix 由现存 wineserver
的 IPC inode 与 `native.verified.local.json` 匹配，不能默认使用另一个 native.local
profile 的 prefix。验收未重启或停止现存客户端与正常服务端。当前未取得原生 query4／分页
请求，不将正常单人 Bot 对局当作公共列表或真人加入通过。

- `fixtures/protocol/room-browser-synthetic.json`：本版消费者地址、独立 Python
  tagged 编码生成的普通／分页／入房请求，以及合成创建描述与列表记录；只含
  虚构 UID、游标与零密钥。
- `research-output/ghidra/public-room-*.c/.asm/.log`：本版只读反编译与指令导出。
  接入依据使用上述有确定边界的函数；Ghidra 在任意操作数字节上误建函数的输出
  不作为语义证据。
- 前端依据：`research-output/remaining-map-analysis/frontend-source/scripts/codo/`
  下的 `tasks/ListRoomTask.as`、`tasks/JoinRoomTask.as`、`services/MatchService.as`、
  `controllers/MatchController.as`、`views/MatchSelectPvpView.as`。

复现主要静态链：

```sh
.tools/ghidra/ghidra_12.1.4_PUBLIC/support/analyzeHeadless \
  research-output/ghidra codol3333 -process codoMP_client_shipRetail.exe \
  -readOnly -noanalysis -scriptPath tools/ghidra \
  -postScript DecompileTargets.java research-output/ghidra/room-browser-check.c \
  6acf70 6e3200 6e3310 6d4810 a570d0 702520 7d0550 a558b0 \
  a55700 701db0 68c790 6c9fe0 a4db00 a4ca40
sh tools/cargo.sh test -p codol-protocol --locked room_browser
sh tools/cargo.sh run --locked -p codol-server --example room_browser_acceptance -- \
  research-output/room-routes-acceptance.json
```

业务验收命令在任一目标未满足时返回非零，输出实际 handler、错误码与结果数量；
临时资料与动态端口在结束前清理。不会操作现存客户端、正常账号资料或正常端点。

真实大厅展示、跨客户端加入、非首屏续页、密码／邀请及房主迁移仍须按上面的独立
验收闭环确认。当前路由实现不把这些原生验收与共享对局项目标为已通过。
