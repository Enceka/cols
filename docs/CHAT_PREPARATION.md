# 大厅、房间与游戏内文本聊天接入准备

目标为 CODOL 3.3.3.3 的公共大厅聊天、房间全体／队伍聊天及游戏内全体／队伍聊天。当前已实现服务端公共聊天路由、房间 type2 解析和队伍范围过滤；原生客户端聊天 UI 尚未完成验收。角色语音、Party、私聊、战队与喇叭不在本专题范围。

## 当前结论

| 场景 | 本版客户端路径 | Rust 当前行为 | 修复重点 |
| --- | --- | --- | --- |
| 大厅公共频道 | UI频道0 → `102/1`订阅变更、`102/3`发送 → SDK event34接收 | Rust已实现订阅、校验、event34跨连接投递；rank1账号已实采`102/1`订阅130/99，运行中的客户端已实采到event34定向推送；本版客户端既无公共频道发送入口也无渲染位置 | 服务端已覆盖；原生发送与显示需要客户端改动 |
| 房间全体 | UI频道3 → 房间type2记录 → `11/4` → event7 | 服务端已解析并转发全体文本；普通成员控制记录仍受白名单限制 | 双客户端原生发送与接收 |
| 房间队伍 | UI频道5 → 同一type2记录，team位为1 | 服务端已按当前成员队伍过滤；普通成员发送仍需原生房间进入验收 | 双客户端切队及实机发送与接收 |
| 游戏内全体／队伍 | 同一房间消息的接收链进入聊天控制器；对局状态另有队伍过滤 | 无独立文本聊天语义或对局成员路由 | 保留大厅连接，使用已绑定的房间／对局范围；实采游戏内发送入口 |

房间文本已经有可复用的加密传输和event7外层，不需要重写大厅传输。仅在UDP里处理`say`不会覆盖这里已定位的原生聊天链路。

## 本版依据与置信度

所有原生地址均为固定样本的 **VA**，ImageBase为`0x400000`。样本SHA-256为
`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`，与
`fixtures/client-3.3.3.3.json`一致。原始指令用于核实反编译中丢失的寄存器参数；不能直接按反编译变量名推导字段含义。

| 位置 | 本版静态证据 |
| --- | --- |
| `0x6b14a0`，vtable `0xc71214` | `SendChatMessage`绑定至`0x5e4760` |
| `0x5e4760` | 频道0调用公共组发送；频道3／5调用房间对象vtable+`0x40/0x44` |
| `0x6ace80 / 0x6acf10` | `JoinNewbieChannel / LeaveNewbieChannel`进入公共组订阅变更 |
| `0xa4d500` | 构造`102/1`，依次写两组数量及组标识数组 |
| `0xa4c730 / 0xa4c770` | `bdMessagingGroup`标识序列化／反序列化：tagged U8类型、U64组ID |
| `0x4a83c0 → 0xa4d810` | 公共发送的内层双Blob长度及外层`102/3`构造 |
| `0xa42db0 → 0xa3ebf0` | event34／35读UID、String(cap64)、Blob(cap1024)，转入不同回调 |
| vtable `0xc7e18c`的+`0x68/0x6c` | `dwLobbyEventHandler`的event34／35回调为`0x6d2060 / 0x6d21e0` |
| vtable `0xc6fa74`的+`0x68/0x6c` | `CommunicationEventHandler`的公共聊天／公告消费为`0x5cc970 / 0x5cc9a0` |
| `0x6dd2a0 / 0x6dd360` | `dwRoomMessageText`的写入／读取，type2、字符串、64位计数、team位 |
| `0x6e8f10`，type2分支`0x6e8fa7` | 房间文本接收、来源成员检查和队伍过滤 |
| `0x5ccc00 → 0x5e9fc0 → 0x5e92c0` | team位映射为UI频道3／5，调用`ChattingController.cppOnMessageReceived` |

前端依据来自已提取的本版`codo.gfx`：`codo/constants/ChattingChannelType.as`、
`codo/controllers/ChattingController.as`和`codo/views/FsgGameLobbyView.as`。
原生字段与调用关系为高置信度静态证据；原生客户端显示、实际投递和双客户端对局聊天仍待动态验证。

## 公共大厅协议

UI常量为`NEWBIE=0`、`PARTY=1`、`CLAN=2`、`ROOM=3`、`PRIVATE=4`、`ROOM_TEAM=5`。
这些值与SDK组类型、组ID、房间ID不同，不能直接混用。

`102/1`参数为：

```text
U32(remove_count)
  remove_count × { U8(group_type), U64(group_id) }
U32(add_count)
  add_count × { U8(group_type), U64(group_id) }
NoType
```

`JoinNewbieChannel(x)`传入remove_count=0、add_count=1；
`LeaveNewbieChannel(x)`传入remove_count=1、add_count=0。
因此`[0,1,130,10]`不能解释成page/count查询。SDK还绑定了可选`bdMessagingGroup`结果数组，
每个元素读U8和U64；成功／部分失败时返回哪些元素需结合动态回执确认。

`102/3`参数为`U8(group_type), U64(group_id), Blob(payload), NoType`。
本版公共发送分支固定使用`(130,10)`。payload来自关闭tag检查的内层ByteBuffer：

```text
LE32(first_blob_length = 0)
LE32(message_byte_length)
message_bytes
```

`ChattingController.encodeMsg`在公共消息前添加`"tUtoR" + "0"`；
`decodeMsg`查找此标记、处理导师位并剥离前缀。`tUtoR0`属于消息内容的频道标记，
不能当作发言者昵称，也不能依据它切出或认证发送者。中文长度按编码后的字节数计算。

event34外层为`U32(34), U64(sender), String(sdk_name), Blob(payload)`。
`0x6d2060`读内层两个Blob，解码SDK名称，再经`0x5cc970`进入UI频道0。
event35进入`0x5cc9a0`公告路径，不能替代公共聊天。
发送与接收消费链支持“event34封装原始双Blob”的候选格式；该格式已在本版原生大厅连接上投递成功，客户端未出现解密或序号错误。
公共UI回调将senderID设为0；自身显示是否需回送、是否与本地事件重复，必须实采确认。

原生实采与仍需验证的行为：

`research-output/chat-ui-20261028d`中prestige=0、level=1的账号发出
`102/1`，参数为`U32(0)`、`U32(1)`、`{U8(130), U64(99)}`、`NoType`，
帧体25字节`66 03 01 08 00000000 08 01000000 03 82 0a 6300000000000000 00`。
订阅只出现一次，未观察到另一条订阅组10的路径，而公共发送分支固定组10，
因此两个ID指向同一公共频道：回执按客户端订阅的ID原样返回，
投递时把99与10当作同一频道比较。

`CareerController.isInNewbieChannel`（prestige=0且level<19）决定客户端是否订阅，
验收需使用满足条件的独立测试账号，不能更改用户正常账号等级。

`102/3`在本版客户端没有可达入口：`FsgChatWindow.onGetAllChannels`只处理`ChattingChannelType.PRIVATE`频道，`postMessage()`只对`PARTY_TAB_INDEX`、`CLAN_TAB_INDEX`和已选好友发送；新兵频道虽然由`addChatChannel`加入`_chatChannels`，但没有任何页签或输入框引用它。大厅聊天按钮（`FsgSocialPanelView.CHAT_BTN_INDEX=5`）启动时由`switchEnableButton(...,false)`置灰，只有`EVENT_UPDATE_UNREAD_CHAT`计数大于0才恢复，而该计数依赖先收到消息，所以没有第二个发送者时无法驱动这个界面。游戏内聊天走房间type2与`11/4`，已实采team位0与1的记录，同样不是`102/3`。公共发送的服务端行为由`chat_transport_tests`的双身份加密传输测试覆盖，跨连接投递另有脚本发送者实采。新兵频道消息只写入`_chatChannels`，而`DataID.CHAT_CHANNELS`的唯一消费方`FsgChatWindow.onGetAllChannels`只处理私聊频道，因此本版大厅没有渲染公共频道文本的位置；原生显示验收在本版客户端上无法完成。

## 房间文本协议与成员范围

`11/4`为`U64(channel), Blob(batch), U32(delivery), NoType`；
`11/5`额外带定向接收者UID。当前批次由`LE32(record_bytes)+record`组成。
原生record采用LSB-first位流：

```text
1 bit: 0，关闭tag检查
8 bits: type = 2
8 × N bits: 消息字节，末尾NUL
64 bits: QueryPerformanceCounter计数
1 bit: team_only
```

写入函数`0x6dd2a0`调用`0xa6e160`取得QueryPerformanceCounter；
这个U64不是玩家UID、房间ID或Unix时间。接收方从event7外层读取真正的发言者UID与SDK名称。
记录可嵌套于type1 manifold，所有字段从第1个位偏移开始，不能按整字节切片。

当前服务端仍存在四个明确缺口：

- `room_messages::validate_participant`已接受合法type2并校验有界文本、计数器和team位；普通成员的其它房主控制记录仍会被拒绝。
- `room_directory::peer_send`已按发送者和收件人的当前room team过滤team文本；混合状态批次和共享游戏成员表仍需原生验收。
- `Human::owner`默认将UID字符串编码为SDK名称；创建／更新房间描述只补齐房主昵称。加入者昵称尚无完整来源。聊天显示和对局队伍的按名字检查都需要核对可信昵称来源及UTF-8／GBK转换。
- type1可以混合聊天与准备／角色／房间状态。过滤整批消息会丢状态，广播整批消息会泄露队伍文本。应按子记录及目标分发，保持同一发送者的消息顺序。

原生房间接收会用发送者UID检查成员，并仅将队伍文本显示给同队；
对局状态值10时，`0x6e1600`按来源名字查找clientinfo再比较队伍。
当前Rust传输对已识别的type2队伍文本执行服务端范围过滤；其它混合记录仍按原生房间状态路径处理。
房间team1／2与游戏team1／2的顺序相反，已存在映射`game_team = 3 - room_team`。
无队伍、观察者、尚未完成加入的成员不得因为team值相同而自动成为队伍收件人；具体UI行为需要实机确认。

## 游戏内接入边界

本版房间文本已经走到游戏使用的聊天控制器，且原生接收显式区分对局状态；
Alpha参考的游戏聊天也由房间type2送入大厅通道。下一次动态采集应同时观察TCP`11/4`及UDP可靠命令，
用实际HUD的全体／队伍发送各一条消息确认本版输入，不以参考版本地址或`say_team`名称替代证据。

Rust当前每个`game_connection::Connection`分别拥有`TdmMatch`，
`backend/rooms/mod.rs`对多真人房间开局明确返回`shared_game_world_not_available`。
公共聊天可跨大厅连接实现，房间聊天可用已有真人成员投递；完整双真人游戏聊天验收仍需共享对局成员绑定。
同一UDP监听地址不代表同一局，不能用监听端口或全局在线连接列表决定游戏全体频道。

若同一局的玩家保留不同大厅房间，需要独立对局成员表及玩家到当前接收房间的映射。
只允许抽取并投递聊天记录，不能把开局、改图、准备或其他房主记录跨房间广播。
当前单真人＋Bot场景只验证自身显示与生命周期，不构成多人聊天完成的证据。

## 建议修复顺序与验收

1. 在protocol中恢复type2与manifold解析，验证有界字符串、UTF-8、NUL、尾部位数和长度。保留真实来源UID，允许已接纳成员聊天，继续拒绝成员生成房主控制记录。
2. 在Rooms中统一单真人／多人发送分支，按room team过滤，复用`DirectedPush`与现有加密writer。补齐可信昵称；发言者回送和客户端本地显示以实采结果确定。
3. 新增公共组订阅状态和`102/1、102/3`业务，处理断线、过期与被替换登录的清理。确认实际频道ID及event34后实现跨连接广播。
4. 绑定真实游戏成员／队伍并接入聊天路由，原生验证开局、死亡、观战、结算返回、切队及换房后的频道范围。

| 验收场景 | 必须观察到的结果 |
| --- | --- |
| A与B在公共大厅，C未订阅 | A/B各显示一次；C无消息；频道ID和昵称一致 |
| 三人同房间，另有独立房间D | 房主及普通成员均可全体发言，D无消息 |
| A/B同队、C另一队 | A/B的队伍文本只发给A/B；切队后立即按新队伍生效 |
| 聊天与准备／角色更新混合批次 | 聊天范围正确，房间状态同步完整 |
| 中文、英文、空白、引号、长消息、截断及非法嵌套 | 编码与边界正确；失败不破坏后续RPC与推送序号 |
| 空闲收件人、断线重登、被替换账号连接 | 空闲仍收到，旧连接和旧房间不再收到 |
| 两局同时运行、开局及结算返回 | 不跨局；游戏HUD和返回的房间频道均正常 |

## 已完成的运行验证与复现

独立临时服务目录、合成UID42/43/44、真实当前`LocalServices::handle`复现：

- 合法type2全体及中文队伍记录：房主成功，回送1条、跨连接投递给43和44；普通成员均被白名单拒绝。
- 四次真实客户端启动验收：AUTH、协议210、大厅注册和`71/3、71/7、71/9`均成功；rank70账号三次运行均无`102`帧，rank1（prestige=0）账号运行发出`102/1`订阅130/99且未产生`102/3`；客户端窗口保持黑屏，聊天UI未被驱动。
- 房间协议测试7项、房间目录测试9项及双身份TCP传输测试3项通过（含公共聊天投递1项）；1项原生type18采集测试按既有条件ignored。
- 脚本发送者实采（`crates/server/examples/chat_sender.rs`）：UID777取得本地票据、建立加密大厅连接，`11/11`因频道不存在返回失败但昵称已登记，`102/1`订阅130/99返回1条记录，`102/3`发送返回`public_chat_message`成功；同一实例上运行的rank1原生客户端随即收到两条定向推送`mailbox-000000-s2c.bin`、`mailbox-000001-s2c.bin`，此后继续发出21帧RPC（最后一条为`local_cp_balance`），加密序号未失步，进程保持`frontend_state=5`、`connection_state=0`。
- 本机无法取得原生像素证据：`CGPreflightScreenCaptureAccess()`为false，`screencapture`与`game-window capture`均失败，`PrintWindow`/`BitBlt`读不到D3D9表面（整幅为黑）；改用代理`d3d9.dll`截取后备缓冲的方案在Wine下没有触发`CreateDevice`/`CreateDeviceEx`，已放弃，HUD显示仍以客户端状态探针与协议帧为准。

这些测试只证明当前代码路径及消息字节分发，不证明队伍UI显示或原生聊天验收。
本机聊天元数据保存本版地址依据与合成向量，原生接收候选未动态验收。原始指令、`native-summary.json`和诊断结果位于忽略目录；从内部引用位置误建的函数不能作为语义证据，有效入口以指令和调用链为准。

```sh
sh tools/cargo.sh test --locked -p codol-protocol room::messages
sh tools/cargo.sh test --locked -p codol-server --lib rooms::directory_tests
sh tools/cargo.sh test --locked -p codol-server --lib transport::lobby::server::room_transport_tests
sh tools/cargo.sh test --locked -p codol-server --lib transport::lobby::server::chat_transport_tests
sh tools/cargo.sh build --locked -p codol-server --example chat_sender
./target/debug/examples/chat_sender --auth 127.0.0.1:39002 --lobby 127.0.0.1:39001 --user 777 --text hello
.tools/ghidra/ghidra_12.1.4_PUBLIC/support/analyzeHeadless \
  research-output/ghidra codol3333 -process codoMP_client_shipRetail.exe \
  -readOnly -noanalysis -scriptPath tools/ghidra \
  -postScript DecompileTargets.java research-output/ghidra/chat-recheck.c \
  5e4760 6ace80 6acf10 a4d500 a4d810 6dd2a0 6dd360 \
  a3ebf0 6d2060 6d21e0 5cc970 5cc9a0 5ccc00 5e9fc0 5e92c0
```

AlphaBuild的`lobby/docs/service102_chat.md`和`dw_rooms.py`可用于寻找线索；
本版已核实的`tUtoR0`标记、订阅变更以及U64性能计数语义应按上述消费与生产代码处理。
