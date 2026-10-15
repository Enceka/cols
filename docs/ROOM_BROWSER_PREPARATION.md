# 原生房间列表与多人入房路由

目标为未修改的 3.3.3.3 客户端与纯 Rust 服务端。客户端主程序 SHA-256 为
`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。
下面的地址均为该 PE32 样本的 VA，不能套用 AlphaBuild 的 C1 地址或结构。

公共房间浏览、稳定分页、认证成员加入／退房、配装／队伍推送与共享对局已接入。两个原生客户端完成同房、真人替换Bot、共同进图、交火、结算和再开局，见[多人验收](MULTICLIENT_ACCEPTANCE.md)。本页保留本版线格式与路由约束，完整模式和装备不由核心联机证明。

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

## Rust 接入与验收边界

房间业务由 `backend/rooms` 维护权威成员和预留，`transport/lobby` 做请求／推送编码，`game/session/shared` 持有一局世界。每个真人保留独立准入、输入、ACK和复制历史，初始配装从各自账号读取，不能将单人世界分别启动后称为共享对局。

实际原生条件为 `[build, 0, 16, 0]`，第三项与创建描述的兼容性字段精确匹配；已核对的基础查询形状按各自校验处理，不忽略未知条件。房间列表只统计不可替换真人席位，内部名单显示真人和Bot总人数。`mmrsv`预留要求双方仍在线，并有30秒期限。

原生显示、加入、真人配装和一致结算已验证；共享多人中途配装、个人回放、经验和完整装备仍需专项验收。远端Linux联调通过SSH转发完成，不等于公网UDP直连，见[远端验收](REMOTE_LINUX_ACCEPTANCE.md)。

最小独立向量为 `fixtures/protocol/room-browser-synthetic.json`，身份／密钥均为合成值；该向量只证明格式。原始请求、反编译及原生运行结果留在本机忽略目录，不随分发提供。
