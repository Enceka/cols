# 原生选图、房间与结算

客户端为未修改的 3.3.3.3，运行核心为纯 Rust。用户选择人机自建房及原生箭头选图，11 个模拟成员使用 `Bot 1`～`Bot 11`，昵称不绑定阵营。正常入口为 `python3 tools/play.py`，使用 `wine10-throwables-20261011`，默认持续至退出。

## 原生房间

创建入口为原生“战术团队竞技”。本版 `coGameTypesTable.csv` 中 `combat`／`train` 的展示标志关闭，因此创建列表没有独立“人机对战”条目。快速人机赛单使用客户端投票界面，不能作为自建房箭头选图的验收。

`--native-bot-rooms` 启用本地模拟成员。支持本版、12 人、自建 `war` 模式和已有资源准备的地图；原生 minimum=4 由真实入房成员满足。频道和房间归属必须匹配当前认证用户、title 和 session seed。

创建 RPC 返回时，原生房间接收器尚未安装，因此服务端等待房主首个频道广播再加入 Bot。首次广播不一定含 kind15 RoomInfo：没有选图记录时，使用创建描述中原生 map/mode CRC、playlist=-1、容量及 build 确认地图；收到 RoomInfo 后以最新记录为准，不用旧创建 CRC 覆盖新选图。这样直接创建到已准备地图无需再切图，未准备地图仍不借用其它图资源。

每个 Bot 先发送 SDK event9 成员／频道元数据，再发送原生 JoinRequest 与 Ready。`0x6e8a20` 要求发送者存在于 SDK 频道表；SDK 名称使用 Base64，局内／结算名称为明文，战队标签为空。`bots_announced` 仅表示加入消息已排队，真实名单仍由房主处理后确认。

服务端消费 `11/5` 及 `11/4` 定向 manifold 中的 JoinResponse／完整成员表。type13 请求 type14 名单；房间阵营1=Allies、2=Axis，与游戏快照枚举顺序相反。准备开局时冻结实际6v6名单，驱动出生、友敌判定、Bot服装、击杀计分和AAR。换图／结算返回会重置准备状态；玩家再次准备才重新设置 Bot 准备状态，不自动连续开局。

## 地图准备

| 地图 | 原生 map | 本图 NCS CRC |
| --- | --- | --- |
| Afghan | mp_afghan | 523897474 |
| 安全屋（本版显示“杀戮小屋”） | mp_killhouse_sh | 3852334052 |
| 坠机之地 | mp_crash_tropical | 4159215889 |
| Bog | mp_bog_sh | 3785594714 |

四图具有独立实体、碰撞、TDM出生点、HUD、武器与角色资料，正常准备为7500分／600000ms。原生箭头仍可选择其它地图，但未准备地图必须明确拒绝。赛单轮换按 publisher version／playlist／entry 顺序解析；自建房按描述和受限游戏请求识别地图，不执行 `mmstart` 文本。正常 publisher 为 version7。

本版26条团队竞技地图的真实ID、模式和角色规则已从`coMapList.csv`生成身份表，运行时准备及结算不再局限于三个手写ID；仍逐图要求完整准备文件。资源列表中23张图观察到主图／load／const路径，当前完成上述四图的实际准备。覆盖及证据边界见 `MAP_COVERAGE.md`。

坠机小地图使用实际 `compass_map_mp_crash_dlc` 材质及1024×1024贴图，角点为 `(1976,2528)`／`(-1200,-2166)`。`compass_map_mp_crash_tropical` 在本版指向16×16默认贴图，不能用作底图。探针按 `0xaa3650` 的材质比较判断 `render_default`，不将资源表的其它布尔字段当作渲染默认判据。

初次出生和重生使用同一胶囊贴地校验。结算 PM_INTERMISSION6 不加 view-height，服务端以眼高设置其视点，同时保留角色脚底原点，重复快照不累加高度。当前完整回放及前三名能力和限制分别见 `KILLCAM_PROGRESS.md`、`TOP_PLAYERS_PROGRESS.md`。

## 生命周期与证据

正常运行将认证业务帧用于续期同 title／owner／seed 的现存房间及 UDP 准入，不延长未认证心跳、不恢复过期或撤销记录。新连接仍要求有效票据。日志采集上限不限制服务寿命。

| 验证 | 证据及范围 |
| --- | --- |
| 直接创建补齐 Bot | `room-initial-bots-live-proof.json`：`client-1791695796942` 直接创建 `mp_crash_tropical`，未点击地图箭头，房间和频道均12人、实际6v6名单确认；随后进入state10，本图CRC4159215889，Bot正常交战 |
| 创建顺序及拒绝边界 | `room-initial-bots-before-valid.log` 保留缺少加入消息的基线失败；`room-initial-bots-delivery-check.log`：355 Rust／50 Python及fmt/Clippy通过，覆盖创建回调前不抢发、首次广播补齐、重复广播、身份、未知地图、非war、非自建、容量和新选图优先 |
| 安全屋胜负结算 | `client-1791562129698` 的 `user-custom-safehouse-win.jpg` 与 `client-1791562581781` 的 `user-custom-safehouse-defeat.webp`，玩家胜／败及两队统计与服务端一致 |
| 名单及连续开局 | `client-1791565199316` 两局及离房重建第三局，房间转换后的阵营逐项匹配AAR；编号昵称和空战队标签已实读 |
| 跨图及小地图 | `client-1791567358365` 坠机／安全屋切换的独立CRC与AAR；`client-1791610820978` 的材质读取和 `ui-1791612112814.png` 确认坠机底图可见 |
| 玩家移动和武器同步 | `minimap-final-live-results.json`：首次出生前进／侧移的当前权威与预测位置、连续正反滚轮后的武器状态一致；不是逐帧动画验收 |
| 原房间长时间再开局 | `continuous-lease-reopen-proof.json`：启动2153秒后原12人房间再次进入坠机state10，跨过原930秒准入期限；不等于无限时稳定性保证 |

只读位置诊断应使用 `current_*`／`predicted_*`；`full_snapshot_*` 是初始完整基线，连续 delta 时不代表当前位置。AAR public buffer 开头有4字节CRC，字段读取必须包含这一偏移。

尚需扩展更多模式／地图、自动资源准备和恢复、小时级稳定性及完整逐图演出验收。体验复核统一列于 `REVIEW_PENDING.md`。

原始采集、账号和用户截图保存在忽略目录。研究副本的已知故障 IIPS／IFS 对象保留于 `blocked-resource-1791544530`、`blocked-resource-1791561992`；不要重新打开这些故障对象。替换副本的内容哈希与原始样本一致，原件保持不变。
