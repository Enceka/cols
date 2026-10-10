# 原生选图与失败结算

2026-10-09，客户端 3.3.3.3；运行核心为纯 Rust。

## 已验证

- `client-1791546282817`：用户投第二张地图后进入 `mp_killhouse_sh`。原连接阻塞来自服务端只接受 `start_map_rotate_once 0`；真实请求携带 `start_map_rotate_once 1`。现在通过 catalog 的 publisher version / playlist / entry 顺序绑定准备文件，不执行 mmstart 文本。
- 本局在 7799 ms 由 Axis Bot 5 击杀 Allies Bot 5，比分 `[0,100,0,0]`，winning_team=1。用户提供的原生结算图显示 141 特遣队 **0 / 失败**，暗影军团 **100 / 胜利**，双方各六人；玩家昵称、击杀和死亡记录匹配。失败结算已有实机证据。
- 用户先前的半屏天空图发生在结束阶段。C1 `0x51d3b0` 在 PM_INTERMISSION6 时跳过 view-height 偏移。现在仅将结算 PS 的视点提升到眼睛位置，保留角色的脚底/碰撞原点；重复快照不会累加高度。回归已通过，视觉修正仍需新的实机结束阶段核对。
- `client-1791553233023` 的 `ui-1791554681642.png` 记录原生自建房地图箭头；只读探针读取 `mode=war, map=mp_killhouse_sh, playlist=255`。截图中的本版原生地图名为“杀戮小屋”。

## 用户选定的方向

用户要求减少截图，并明确选择 **人机自建房，沿用原生箭头**。不再把赛单投票当作箭头选图完成。

当前 publisher 源为 version 7，撤回第二张轮换图并加 novoting。`--native-bot-rooms` 同时启用服务器实验房间策略和客户端原有最低人数 dvar。

## 2026-10-10 频道入场修复与实机进展

- 新参考文档为工作区 `CODOL-2.6.9.7-技术文档.md`。只参考行为和验证方法，本仓库继续使用 3.3.3.3 原生消费者及纯 Rust 后端。
- 上一轮“消息在接收器上层被丢弃”的结论不准确。`0x6e8a20` 在回复 JoinRequest 前查询 SDK 频道成员表；发送者不在表内时直接返回，成功和拒绝回复都不发。故单看 11/4 无回复不足以证明消息未到接收器。
- 已补 event 9 频道加入通知：`0xa3d730 -> 0x6d1930 -> 0x6c9ba0`。先发布 version 2 的成员/频道元数据（保留真实房主），再发 JoinRequest/Ready。等待房主 RoomInfo 广播后入场，不在创建 RPC 回调前抢发。`bots_announced` 只表示通知已排队。
- `client-1791562129698` 的只读状态 `client-state-1791562169637.json` 确认 `room_member_count=12`、`room_channel_member_count=12`、`room_is_host=1`、`playlist=255`、`map=mp_killhouse_sh`。用户箭头选图后成功开局、结算和返回房间。用户结算图 `user-custom-safehouse-win.jpg` 显示玩家队 100/胜利、敌队 0/失败；服务端 8200ms、winner=2、Allies Bot 4 击杀 Axis Bot 1 一致。
- 用户报告房间昵称乱码，名片中 `[Allies Bot 3]` 出现在战队标签位置。`0x6d3830` 会对 SDK 名称做 Base64 解码；旧实现发送了明文，并把昵称错误写入 JoinRequest 的 clan tag。已分别改为 SDK Base64 名称和空战队标签，待下一次实机核对。局内/结算仍使用明文名称。
- `native-channel-membership-check.log`：280 Rust 测试、39 Python 测试、fmt/Clippy 通过；3 个诊断测试按原配置忽略。昵称修正也通过同一完整检查，见 `native-room-name-check.log`。
- 本次再次保留并替换被卡住的研究 IFS 文件对象，原始内容 SHA-256 不变。证据 `blocked-resource-1791561992/repair.json`；不要重读故障对象。
- `client-1791562581781`：只读成员表确认全部 11 个 Bot 昵称正常、战队标签为空。原生箭头切到 `mp_killhouse_sh` 后，用户再次开局并提供失败结算图 `user-custom-safehouse-defeat.webp`：玩家队 0/失败、敌队100/胜利。服务端7003ms、winner=1、Axis Bot 6击杀Allies Bot 4，比分完全一致。此新自建入口的失败结算验收通过，客户端也已返回原房间。
- 用户要求后续自主测试、尽量少截图，并同意统一使用 `Bot 1`～`Bot 11`。`client-1791564120600/client-state-1791564231793.json` 已确认这些昵称、空战队标签以及11个Bot准备状态为1。本版实采未找到`mp/botNames.csv`；AlphaBuild的`dis/botnames.txt`是另一套英文列表，未当成本版官方昵称使用。
- 正在补齐房间阵营沿用：成员`+0x88`是房间阵营（1=Allies，2=Axis），`+0xa8`是party，之前探针/JoinRequest注释解释错误已纠正。服务端通过私聊`11/5`消费真实JoinResponse，并用type13请求type14完整成员表；原生JoinResponse结果只有8位（`0x6de190`），其后立即是RoomInfo。局内出生、Bot阵营、计分和AAR改为使用房主确认的完整阵营数组。新的实机验证仍在进行。
- `native-room-roster-final-check.log`：284 Rust、39 Python、fmt/Clippy通过。运行游戏期间observe的500ms idle测试复发，停止实验后的完整检查通过；未放宽该测试。
- `client-1791565199316` 自主实测通过阵营同步。完整名单实际通过11/4的定向manifold回复给Bot 11，不能只在11/5找；已修正并加入脱敏字节夹具。房间游戏阵营转换后为 `[1,2,1,2,1,2,2,1,1,2,2,1]`，服务端出生/伤害记录和客户端AAR每一项一致（`client-state-1791565407067.json`）。两次安全屋开局分别在8649ms和7649ms结束，均返回房间。Bot昵称均为编号，不绑定阵营。新增回归后共285 Rust、39 Python通过。
- 只读AAR探针此前遗漏public buffer开头的4字节CRC，现已修正读取基址；旧探针的map/mode/winner值不能作为结算证据。服务端J字段偏移与生产结算正确，无需修改其数据。
- 同一实验已完成离房重建后的第三局：新房间阵营 `[1,2,1,2,2,1,1,2,1,2,1,2]`，7700ms结束，AAR再次精确匹配。原生箭头遍历可选其它地图；本轮未能在循环中选到Afghan，未据此宣称双图切换已通过。选择尚未准备的`mp_crash_tropical`会明确拒绝，不下发其它地图的数据。

## 仍需完成

真实 `mp/coGameTypesTable.csv` 共 36 行。`combat` 行的三个展示标志均为 0，`train` 行同样关闭。因此原生“创建房间”没有独立的人机条目；当前可见入口仍为“战术团队竞技”。原生 war 创建数据中的 minimum=4 仍会显示“还需 3 名玩家”。

实验 Bot 房间保持 opt-in。频道成员缺失造成的 1/12 阻塞、昵称、阵营沿用、重新开局及新入口玩家失败结算已按上述证据完成。原生最低人数仍为 4，由实际 Bot 成员满足。结算相机已验证眼高数据修正，视觉表现未另做完整镜头验收。

`client-1791553233023` 的另一开局请求为 `map za_coliseum`，因此被地图准备校验拒绝。该图没有本轮资源准备；不能用 Killhouse/Afghan 的 CRC 冒充它。

Afghan/Killhouse均有独立CRC、实体、碰撞、HUD和TDM出生点准备。安全屋完整链路已通过，后续功能对齐从 `REFERENCE_ALIGNMENT.md` 接续，当前正在采集第三张图坠机之地。

## 本地证据

原始采集、用户截图和账号资料均留在忽略目录：

- `research-output/gamehub/client-1791546282817/user-native-defeat.jpg`
- `research-output/gamehub/client-1791546282817/user-intermission-floor.jpg`
- `research-output/native-voted-map-check.log`：275 Rust、39 Python、fmt/Clippy 通过。
- `research-output/native-custom-room-manifold-check.log`：后续新增房间/相机回归通过；对应 release 构建日志为 `native-custom-room-manifold-release.log`。
- `research-output/native-room-selection-final-check.log`：活跃实机期间 observe 的 idle-budget 集成测试再次偶发失败，尚未将这一轮标作完整通过。已有历史同类现象，须在隔离环境复核。
- 原生消费者导出：`native-room-receiver.c`、`native-channel-peer-push.c`、`native-room-options-codec.c`、`native-room-member.c`；均来自此版未修改的客户端。

之前还复现并恢复了研究副本的 IIPS 文件打开阻塞。故障对象保存在 `research-output/blocked-resource-1791544530/`；新对象与原始样本的 SHA-256 一致，不要重新打开保留的故障对象。


## 2026-10-10 小地图、出生点与连续游玩收口

- 小地图根因由本版原生函数与实采共同确认：`0xaa3650` 比较材质 `+0x58/+0x5c/+0x60`，默认材质会在 `0x438900` 跳过底图绘制。`compass_map_mp_crash_tropical` 实际引用16×16的 `default`，`compass_map_mp_crash_dlc` 引用本图1024×1024贴图。旧探针将 XAssetEntry `+0x10` 误标为 `default`，其布尔值不是可靠的默认材质判据，现改为原生比较结果 `render_default` 和精确贴图名称/尺寸。证据：`client-1791604949812/characters-1791610339045.json`、`ghidra/compass-default-test.c`。
- 准备工具和Rust校验共同使用显式材质映射，Crash保持自己的角点 `(1976,2528)` / `(-1200,-2166)` 和CRC4159215889，不复用其它地图范围。原有两个Crash准备文件已同步，旧的 `_tropical` 材质配置会被校验拒绝。
- `client-1791610820978` 从安全屋切到坠机之地；`characters-1791612112668.json` 确认当前材质为非默认，`ui-1791612112814.png` 确认左上小地图底图可见。安全屋本轮底图也仍可见。
- 初次出生与重生统一贴地；原始Axis点 `[1184,-1960,68]` 在当前碰撞中修正为 z=69.26435。`client-1791614156138` 自动输入验证：前进后x=1318.97，侧移后y从-1960变化至-1839.58，再返回-1946.83。客户端预测和当前权威快照均显示位移，差值约0～1单位。证据汇总 `research-output/minimap-final-live-results.json`。`full_snapshot_*` 是初始完整基线，连续delta时不代表最新位置；排查移动应使用 `current_*` / `predicted_*`。
- 快速滚轮按本版原生顺序打断抬枪，收枪途中选回原枪则立即抬起。代码回归通过；上述实机连续5次正反滚轮后，权威/预测武器均为353，两次间隔读取都为state0/time0。此为状态同步验证，不是逐帧动画录像验收。
- 长时间停留原房间后换图暴露930秒UDP准入过期：原实现只续期大厅房间。现正常模式在认证业务帧之后同步续期房间及同title/owner/seed的存活UDP准入；不延长未认证心跳，不恢复过期或撤销记录。时间驱动回归覆盖跨越原期限、错身份和撤销。正常模式的新连接仍需有效票据。
- `minimap-lease-serial-check.log`：295 Rust、41 Python、fmt/Clippy通过；3个原有诊断测试忽略。并行全量的observe空闲测试再次出现调度相关失败（两条idle_timeout），单独复核和`RUST_TEST_THREADS=1 sh tools/check.sh`通过；未放宽500ms要求，保留两份失败日志。
- 当前本机游玩prefix为`wine10-minimap-20261010`，正常入口默认同步更新；不再默认使用已卡住的旧容器。日志/只读探针为主要证据，本轮只在定位界面与小地图验收时留图。


### 续期修复的实际复核

`client-1791614156138`完成标准坠机局，422749ms结束，Axis7500/Allies7400；在客户端启动2153秒后仍保留原12人房间，再次准备成功进入坠机之地state10，材质仍为`compass_map_mp_crash_dlc`。`continuous-lease-reopen-proof.json`保留这次读数，本次日志没有`DATA without peer`或`admission expired`。证明跨过原930秒期限后的原房间再开局，不表述为无限时稳定性保证。随后主动结束验证进程并由harness恢复证书。

按新版参考文档继续排查射击反馈时，另修正玩家射击友敌筛选的固定阵营遗漏，见`REFERENCE_ALIGNMENT.md`。此前房间/出生/结算的实机通过不等于玩家射击判定通过。
