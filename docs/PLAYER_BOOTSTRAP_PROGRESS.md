# 玩家出生配置、发包时钟与装备

2026-10-07，接续 [快照进展](SNAPSHOT_PROGRESS.md)。已显示场景/HUD；本轮首先解决此前约33秒才有一包输入的问题。

## 发包时钟根因与修复

只读对照 `client-1791346873591/client-state-1791346931462.json` ～ `client-state-1791346936492.json`：maxPackets=30，但真实约5秒内clientRealtime仅34258→34263、commandNumber17→22。`client-state-1791347329127.json` 进一步显示fixedTime=0，frameScale/scriptTimeScale原始bits均为27346044，解为接近零的浮点值。

目标VA `0x460210` 判定发包间隔：`1000 / dvar969.current` 与 clientRealtime−lastPacketTime 比较，当前门槛33毫秒。`0x467a00` 增加clientRealtime；`0x59d500` 将缩放后的帧时长下限设为1毫秒；`0x59d840` 同时按这个近零倍率把实际帧频限至1。因此约33秒才达到发包门槛，并非UDP或Mac网卡吞包。

VA `0x579f30` 从**configstring6415（0x190f）**取得文本，使用 `0xc6b49c` 的 `%i %i %g %g` 解析慢动作起点、时长、起始/结束倍率，但不检查sscanf返回值。之前未下发这一字段，局部变量残留进入 `0x59d2f0` 的时间倍率过渡，再由 `0x59d490` 参与帧时间缩放。只读type表确认6415类别为255，可以普通configstring覆盖。

Rust gamestate现在下发 `6415 = "0 0 1 1"`。没有修改客户端函数、内存或本地dvar文件。

`client-1791347562580` 验证：frameScale bits=1065353216（1.0）；采集之间约2秒，clientRealtime 75752→77793→79807，commandNumber2872→2999→3131。超过3400条native包无解析错误，间隔中位数约40ms，恢复正常数量级。准确速率随前台焦点、帧频及调度变化，仍须持续观察。

这也纠正快照文档中“输入较稀疏，原因待查”的旧边界。物理移动仍未实现；输入频率恢复不能代替权威位置积分、碰撞或装备执行。

## 装备接续

准备从同一地图的NCS category13恢复武器ID，不能使用商城item ID直接填入playerState。当前资源中基础 `m4_mp` 对应索引163，`m9_mp`为180；这些值必须从排序/去重后的实际资源导出，而不是跨版本硬编码。

目标 `0x5a18e0` 每个输入包都从当前playerState生成默认命令：PS+0x340、+0x32c、+0x344分别提供当前武器、offhand、alt；PS+0x10的0x1000标志还会将默认melee实体设为2047。新增武器/玩家标志时必须同步输入base。具体下发字段、后续实测结果在实现验收后补充。

## 只读字段地址来源与用户复现

用户删除了探针中未完成的 `0x079964?0`，这是先前编辑错误，删除正确。另删去两个虽已实测但没有记录推导的武器字段；目前保留这两处删除，下面补齐可审计依据：

| 字段 | 地址推导 | 目标指令/数据来源 |
| --- | --- | --- |
| 完整快照基址 | `0x07996128` | `0x46d667` push 0x3564、`0x46d673` push 0x07996128、`0x46d678`调用memcpy；源为解析完成的snapshot |
| 当前武器 | 基址+`0x340`=`0x07996468` | 已提交PS表fixture第39项 `[832,10,0]`；`0x5a18f7`读 `[edi+0x340]`，`0x5a1901`写默认usercmd+0x14 |
| 装备槽0 | 基址+`0x2a4`=`0x079963cc` | `0x5a4a0a` lea ebx,[eax+0x2a4]，`0x5a4a10`数量15；对应snapshot-sections.c中 `extraout_ECX+0xa9`（int指针，需乘4） |
| clip槽0 key/count | 基址+`0x3dc`/`0x3e0` | `0x5a4230`数组读取 `extraout_ECX+slot*3+0xf7`，读取key后两个手的弹药量 |
| reserve槽0 key/count | 基址+`0x364`/`0x368` | 同函数写 `extraout_ECX[slot*2+0xd9/0xda]` |
| maxHealth / otherFlags | 基址+`0x198`/`0x10` | stats mask位2写PS[102]；PS字段表第54项offset16、codec28 |

完整静态证据仍在忽略目录：`snapshot-entry.c`、`snapshot-sections.c`、`usercmd-target.c`；这些绝对探针地址是基址加结构偏移，并非单独在EXE中存在的字面常量。因此按绝对地址检索反编译文本可能找不到。

用户生成的 `ab-armed.json` SHA-256为 `a9ee98cc8b4b31db6296e5d3e2ab8fef79e75b78bfb89cc2a09b689dedac5b44`，与training-armed完全一致；ab-armed-noammo为 `1772590dc1ca85236dda6ce214ca03325480a5447302c32bb326c40f7077e090`，与training-weapon一致。两个失败实验 `client-1791351259026` / `client-1791351715265` 的游戏UDP事件和snapshot计数均为0，不能用于归因弹药/实体编码差异；均已结束并恢复临时CA。


## 装备与弹药的可复现实验

基础 `m4_mp` 的注册定义在 `0x017f5a40` 指针表中。只读采集发现其模型槽 +0x10/+0x18 指向 `tag_origin`，因此只有HUD武器名不代表枪身模型已装配。`0x46da00` 使用 `** %i %i %i`，由 `0x42e3b0` 解析组合武器。对本地图继承的351个武器，新增ID352、CS5795（5443+352）=`** 163 3 0` 可完成组合注册；这里的3是目标格式参数，尚不对其全部标志位作语义推断。

`client-1791355959837/bootstrap-weapon-1791356052661.json` 验证组合ID352的ammo key=163、clip key=352。`0x42bfa0` / `0x42c1b0` 分别维护定义+0x6b8/+0x6c0。工具只从通过EXE哈希、NCS CRC、武器名称和ID校验的运行时采集导入这两个key。30/90是明确的诊断初始值，**不是已验证的武器容量，也没有权威扣弹/换弹实现**。用户已确认弹药HUD正确。

快照扩展了当前武器、槽0、两手clip计数、reserve计数、maxHealth与玩家otherFlags0x1000。基线另含entity0、eType1、地图来源的坐标；普通帧继承实体基线。类型1字段表为0xc52b68，last index=5（6位），坐标采用-82/-83/-81的完整16位路径。实体不包含完整角色外观，不能作为人物同步完成的证据。

`--viewhands` 从category0解析 `viewhands_player_tf141`（本图ID53），填入PS字段56/+0x148；PS字段9/+0x26c使用idle0加0x400重启位（目标0x422e5c/0x42956f）。组合枪身及手模型可见，但姿态仍异常。**这两项目前是可重现的诊断配置，不是第一人称模型/动画修复**；“第三人称动画套到了第一人称”尚无静态或运行时证据。

准备命令（采集不入库）：

```sh
python3 tools/prepare_game_state.py research-output/gamehub/client-1791341394600/net-constants-1791341887328.json --map-entities research-output/gamehub/client-1791344898487/map-entities-1791345085525.json --weapon m4_mp --compose-weapon --viewhands --weapon-runtime research-output/gamehub/client-1791355959837/bootstrap-weapon-1791356052661.json --output research-output/training-viewhands-state.json
```

## 输入基线与鼠标问题：仍未解决

`0x5a18e0` 从当前快照生成usercmd base，只复制pitch/yaw，roll仍为0。默认武器及melee实体也随PS改变，因此解码不能继续使用全零base。服务端保留最多256条发送快照及其输入base，按客户端message ACK选择最近的对应项；可靠命令消息本身不替换该base。使用最新发出但未确认的角度会污染增量解码。

服务端将收到的short angle回写字段19/18/29（pitch/yaw/roll，codec-87）。这仅闭合角度传输，不构成移动模拟，PS.commandTime仍暂用快照时间。

`client-1791359405951/client-state-1791360025402.json`：catchers=0、pmFlags=0、本地usercmd=[3644,15729]，服务端解码相同，PS角度约20.017/86.402度。但用户移动鼠标后画面无反应。该实验已到900秒自动结束，临时CA恢复。后继 `client-1791360581483` 的只读探针显示灵敏度1、yaw/pitch scale约0.022、game scale约1.20446、aim state ready=1；本地角度与快照角度变化，场景截图却保持相同方向，下一步检查CG预测状态与渲染视角的更新条件。

探针来源：0x45f090读取active+0x35e4..0x35f0双缓冲鼠标delta；0x45f120应用灵敏度和active+0x3610缩放；0x45f260调用0x50b8b0再写live angles；0x45f940转为short角度。0x01a2e7ac是被用于除法的鼠标时间间隔，不能命名为mouse_enabled。新增窗口辅助命令move只产生受PID/焦点保护的鼠标移动，用于复现普通窗口输入。

本轮完整检查通过：91项Rust、19项Python，fmt和Clippy无警告。探针重新编译并在目标版本上执行。测试验证字段及协议边界，不代表视角、动画或AI已经可用。


## 视角冻结根因：commandTime错误推进（已实机恢复）

`client-1791360581483/client-state-1791361426006.json` 将断点缩小到预测阶段：CG两个快照的pitch/yaw都为62.69348/132.03369，commandTime为810150/810200；预测PS却是0/0，CG渲染时间810197。网络接收正常，预测commandTime在客户端最新命令之前就被推进到了未来。

目标 `0x4522b0` 的精确条件在 `0x4523d6`：比较PS.commandTime与最新本地cmd.time，`jge 0x4525c7`。该分支 `0x4525d0..0x4525ee` 将CG+0x16b54c/+0x16b550/+0x16b554的旧角度写回PS+0x14c/+0x150/+0x154。之前每50ms把snapshot.serverTime同时写入PS.commandTime，让这个分支一直成立。反编译证据保留在 `research-output/ghidra/player-command-replay.c`。

修复：快照时间仍按20Hz推进，PS.commandTime只跟随已接收并回写视角的输入时间，未收到输入时保持出生时刻1000；编码拒绝commandTime超过snapshot时间。此bootstrap只应用视角，**不代表命令中的移动、开枪和换弹已执行**。原点、弹药等仍是诊断固定值。

`client-1791361563810/client-state-1791361794147.json`：预测commandTime=131844、CG时间=131844、最近CG快照commandTime=131771，预测角度3.02124/-63.09448，与客户端输入及快照一致。用户明确确认“可以动视角了”，同时确认模型仍不对、无法移动。视角恢复不能作为后两项完成的证据。

检查：完整91项Rust+19项Python、fmt、Clippy通过；追加既有调度测试中的断言，确保无输入时重复快照不会推进commandTime，针对性测试通过。协议测试对照目标字段表确认快照1000与输入925可独立编码，并拒绝未来输入时间。窗口截图已随方向改变，后续工作集中在武器动画/DObj绑定及权威移动。


## 模型实验与弹药状态接续

`client-1791362285874` 单独尝试动画0x401：`client-state-1791362434265.json` 中预测动画和第一人称record.lastAnimation均为1025，但截图仍是异常持枪。该实验未修复模型，源代码已恢复初始0x400。目标0x557390处理idle0/idle1，0x55764d是idle0可以提前返回的分支；0x5587e0构造第一人称DObj/动画树，动画库来自 `0x1a15c7c + weaponId*16`，每手记录来自混淆指针表（槽选择0x07817bb8，xor0x39641402，stride24）。下一步应读取实际动画树、模型组成和骨骼对应，不能再次仅切换idle编号。

只读 `bootstrap-weapon-1791362992924.json` 显示基础和组合M4都使用 `viewmodel_m4a1tech_idle/fire/ads_down` 名称。这是第一人称动画资源名的证据，但没有证明运行时骨骼绑定正确；仍不能把用户提出的第三人称动画解释当成根因。

用户随后发现射击后弹药回到30。原因是快照每次从准备文件重新种入30/90。现在每个连接创建独立Player，保存弹匣/备用弹药、射击冷却和动画重启位。目标0x4253e0将WeaponDef+0x6f0写入weaponDelay、+0xd10写入weaponTime并选择state6；本图基础与组合M4实测为0/79ms。只有经哈希、NCS CRC和武器身份校验的零延迟M4运行时配置启用这段逻辑。过早的未来命令忽略，重复/旧命令不再扣弹，菜单/其他武器输入不触发射击。每条命令最多产生一次扣弹，不补造长时间断包中的射击。

准备文件 `research-output/training-persistent-ammo-state.json` 记录79ms及新采集SHA-256。`client-1791363462590` 已实测弹药0/90：`client-state-1791363622419.json`快照时间123000与 `client-state-1791363845135.json`快照时间345700都保持0/90，没有重置30。前4514条native事件无解析错误。当前仅恢复扣弹与快照一致性，没有命中、伤害、弹道和完整weapon事件系统；用户报告换弹动作出现但仍为0，接下来补服务端换弹与弹药转移。

检查：93项Rust+19项Python通过；第一次在运行游戏时UDP500ms测试超时，游戏正常到期停止后完整重跑通过，未放宽测试超时。fmt/Clippy通过。新测试覆盖79ms冷却、重复旧命令、松开扳机保持弹药、菜单、错误武器、空弹匣及连接间隔离。只读探针另报告当前快照的弹药，避免误读初始完整基线中的30发。
