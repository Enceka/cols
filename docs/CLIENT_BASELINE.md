# CODOL 本地样本预调查

首次检查日期：2026-10-06。以下保留第一轮工作区、PE、字符串和资源基线；同日后续静态及动态补充见文末、[BOOTSTRAP_FINDINGS.md](BOOTSTRAP_FINDINGS.md) 和 [GAMEHUB_DEBUG.md](GAMEHUB_DEBUG.md)。当前已执行 GameHub 中的客户端研究副本并捕获 AUTH 流量，原件未修改。

工作区：`/Volumes/Projects/codol`。检查时 `server/` 为尚无提交的 Git 仓库，没有服务端代码。工作区及其父路径未发现适用的 AGENTS.md。

**版本与环境**

- `Installer/CODOL_Experience_V3.3.3.3_FULL.exe` 存在；首次检查未核验哈希，同日实施已补齐文件指纹。该 EXE 只有 1528836 字节，不能因文件名含 FULL 就断定它包含全部客户端资源；内部内容未检查。
- `CODOL/IIPS/IIPSVersion.ini` 内容为 `3.3.3.3`。
- `CODOL/IIPS/IIPSFilelist.ini` 的 URL 指向 `Experience/V3.3.3.3`。
- `CODOL/TCLS/version.ini` 中 LiveUpdateVersion/TCLS 的版本值为 `2016061600`，属于另一组件的版本记录。
- 在主程序资源区域按 VS_FIXEDFILEINFO 签名读取到的文件/产品版本均为 `0.0.0.0`；该结果不能用于独立确认游戏版本。尚未完整遍历 PE 版本资源树。
- 当前宿主机：Darwin arm64；PATH 中有 `/opt/homebrew/bin/rustc` 和 `cargo`，未执行版本或编译检查。
- `/Applications/Parallels Desktop.app` 存在；未检查虚拟机、来宾系统或应用是否可启动。
- 在 PATH 中未找到 wine/wine64、ghidra、tshark、dotnet、ildasm 或 ilspycmd；不代表其他位置一定没有安装。
- `/Applications/CrossOver.app` 不存在；未搜索其他安装位置。

**关键文件指纹**

| 文件（相对工作区） | 字节数 | SHA-256 |
| --- | ---: | --- |
| CODOL/codoMP_client_shipRetail.exe | 10473472 | `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b` |
| CODO_OfflineLauncher.exe | 46080 | `2af388593f269a721b30735500a1481d341b42a1e8e36f5bb7fb2a6c6b178296` |
| CODOL/IIPS/IIPSVersion.ini | 7 | `3e5d9903a15b50ee297f71606b5ac874eced826b7970c4505285f95b309ddaa7` |
| CODOL/TCLS/version.ini | 134 | `1361cbe830500b3b0663326bec5d8733edfe2e4ae3a40d07c3a34242a0926978` |
| CODOL/IIPS/IIPSFilelist.ini | 64092 | `a1cb0060ed2eddf32135bb38b30110d0d9d79c57ee873d12db2064415601467c` |
| CODOL/IIPS/IIPSVFS.dll | 216120 | `be61df01a23bc3c817b1a4058d588aee0316aa059c0da9e16679365de3e5f449` |
| CODOL/IIPS/IFS2.dll | 601656 | `5b0324e0b2ba3545a6c02626287d4fbc6a3116594744f4647333982d85508421` |
| CODOL/TenProxy.dll | 167592 | `a5ee7ede762c473f58a87b214213097f6dda8950ac537b0a959389aef31348cb` |
| Installer/CODOL_Experience_V3.3.3.3_FULL.exe | 1528836 | `261aa32a40e5bd8f709d1a51bdfa6585d8fe41437b253ea9fa04d8e86b06a739` |

主程序为 Windows Intel 80386 PE32；离线启动器为 PE32 .NET assembly。主程序节名为 `.text`、`.rdata`、`.data`、`.tls`、`.rsrc`、`.reloc`。该布局不足以判断是否加壳、存在运行时解密或保护。

普通导入表含 `d3d9.dll!Direct3DCreate9`、`WSOCK32.dll`、`WS2_32.dll`、`mss32.dll`、`bink2w32.dll`、`awesomium.dll`。尚未检查延迟导入或运行时动态加载，因此不能仅凭此表排除其他依赖。

**主程序的调查入口**

以下位置都是十六进制文件偏移，不是 RVA 或运行时虚拟地址。字符串命中仅证明数据存在，不证明其代码路径可达或协议已恢复。

| 文件偏移 | 字符串 | 后续用途 |
| --- | --- | --- |
| `0x877864` | `dwauth.ini` | 查配置读取及 AUTH/LSG 端点来源 |
| `0x8e6624` | `.?AVbdByteBuffer@@` | 查 RTTI、序列化/反序列化函数 |
| `0x8e81ac` | `.?AVbdLobbyEventHandler@@` | 查大厅事件回调 |
| `0x88458c` | `Unknown bdLobbyErrorCode` | 查错误映射及任务完成路径 |
| `0x86119c` | `getchallenge 0 "%s"` | 查对战连接状态机 |
| `0x8613ec` | `connectResponse` | 查连接响应处理 |
| `0x87b5fc` | `ListDedicatedServer` | 查服务发现；不能据此认定内置专用服务器可运行 |

还发现 `challengeResponse`、`connectReject`、`statResponse`、`disconnect_ack`、`keyAuthorize`、`loadingnewmap`、`uploadStats` 等相邻字符串；存在 `IWff` 字节命中，但未确认对应格式版本。

`dwauth.ini` 邻近位置包含 `stun.ini`、`buildconfig.ini`、`build_version=%s`、`iips.ini`、`enable_iips=%d`、`iips_url=%s`。目前 `CODOL/` 根目录中没有 `dwauth.ini` 或 `buildconfig.ini`，具体文件格式及缺省行为仍需逆向。

**启动器入口**

从启动器提取到以下 UTF-16LE 字符串；没有对它们的调用关系下结论：

- `-q 0 -src=tgp -game_id 29 -zone_id 83886082`
- `codoMP_client_shipRetail.exe`
- `codoMP_client_shipRetailBase.dll`
- `IIPS\IIPSHostApp.exe`
- `Cross\CrossProxy.exe`
- `runas`

`CODOL/codoMP_client_shipRetailBase.dll` 当前不存在。需要通过反编译确认这是替代路径、可选模块或实际缺失依赖。字符串中的参数不能直接当成经过验证的启动命令。

**资源与配置**

检查时 `CODOL/` 下共有 1081 个文件，合计 11065691114 字节；统计包含组件及日志等附属文件，不代表纯游戏内容体积。

`IIPSFilelist.ini` 声明 362 项，解析得到 362 个资源文件名。与 `CODOL/IIPS/IIPSDownload/` 下的 362 个 `.ifs` 文件逐一比较，文件名无缺失或额外项。这不验证文件内容、块缓存、内部校验或地图依赖是否完整。

抽样读取的 `.ifs` 文件头前四字节为 `6e 69 66 73`（ASCII `nifs`）。资源文件名包含 common、marketplace、cargoship、highrise 及其他地图/合作模式线索；未打开其内部内容，不能据此选择最终 MVP 地图。

`CODOL/iips.ini` 中 `enable_iips=1`，并配置了一个内网 HTTP 资源地址；该地址仅作为配置事实，未连接、未验证可用性。

`CODOL/stun.ini` 含多个 `stun*-{stn,tub,ctg,stg}.codol.qq.com` 主机名。未进行 DNS 解析或端口连接。存在配置不代表当前启动路径一定使用它们。

`TCLS`、`Cross` 及相关子目录含 6 个 `.tdr` 文件，尚未判断其格式和业务归属。对 `CODOL/` 的扩展名扫描未发现 `.pdb`、`.pcap` 或 `.pcapng` 文件。

**早期调查假设（后续验证见文末补充）**

| 假设 | 当前置信度 | 验证方式 |
| --- | --- | --- |
| 主程序读取 dwauth.ini 中的两组端点和十六进制值 | 静态+动态已确认 B=HTTPS AUTH、末项=title_id | A 的业务角色与配置覆盖顺序仍待验证 |
| Demonware 风格序列化参与大厅通信 | RTTI/错误字符串线索 | 对象调用链、报文及客户端解析结果 |
| challenge/connect 参与实时对战连接 | 字符串线索 | 状态机分析及真实连接实验 |
| 客户端可在 Mac 兼容层运行 | GameHub 引擎 10 已运行至真实 AUTH 请求 | 完整图形、音频、输入及大厅稳定性仍待验证 |
| 本地 .ifs 足够支持一张完整地图 | 文件名匹配 | 索引/块校验、实际读取及进图实验 |
| 客户端残留可执行的服务器逻辑 | 未验证 | 代码和依赖分析，必要时动态实验 |

这些基线支持制定调查顺序，不构成服务端已经可实现全部玩法的证明。

**同日实施补充**

- 对包括安装入口 EXE 在内的 9 个文件建立机器可读指纹基线；原程序和配置未修改。
- 362 个 .ifs 的文件名与清单匹配，全部检查到 nifs 文件头；已逐文件计算 SHA-256 并保存于 fixtures/resources-3.3.3.3.json。该参考值用来识别后续变动，不代表官方校验或内部资源块完整。
- 对启动器完成 CIL 解码。Base.dll 和 IIPSHostApp 的存在用于分支/通知；启动器直接启动主游戏程序，随后按条件启动 CrossProxy。具体偏移及限制见启动发现文档。
- 在主程序中确认配置读取的五次扫描：`%59s`、`%hu`、`%59s`、`%hu`、`%x`。已记录目标全局地址和后续调用入口；最终字段语义仍未知。
- 本机 Homebrew Rust 因 LLVM 动态链接错误不可用；已单独安装 rustup Rust 1.96.0，原生构建及本地测试通过。第三方静态工具安装于仓库 .venv。
- `prlctl list -a` 显示 Windows 登记项状态为 invalid。未启动或修改该虚拟机。Mac 客户端兼容层实验尚未执行。
- Rust TCP 观测器与合成流量测试已实现；相关记录不能作为游戏登录成功的证据。新的实现状态与验证范围见 FOUNDATION.md。

**GameHub 动态验证补充**

- 用户指定优先使用 GameHub 的兼容层。找到本机 wine-proton_10.0/11.0，引擎 10 已在独立 prefix 中运行本样本，11 只查询了版本。
- 从原件创建独立 APFS 可写克隆后启动，Wine 日志确认读取 dwauth.ini/iips.ini、加载 TenProxy/TerSafe、由游戏启动 IIPSHostApp。原始目录未作为运行工作目录。
- endpoint_b 捕获 70 字节 TLS ClientHello，后续经本地 TLS 1.0 探针得到 POST /auth/、auth_task="10" 的 JSON 请求。配置末项由 0 改为 0x3EA 时，title_id 从 "0" 变为 "1002"。
- 在研究副本的 cacert.pem 中临时信任实验 CA 后，两次握手成功；每次结束均恢复并核对 CA 文件。TLS 传输尚未进入 Rust 运行时，认证响应/票据和大厅仍未实现。
- Parallels Windows 已恢复 running，来宾为 Windows 11 Arm；只核对环境，没有在该虚拟机执行游戏。完整实验目录、限制及下一步入口见 GAMEHUB_DEBUG.md。

**Rust 认证、通道和业务恢复补充（同日后续，替代上面早期状态）**

- 真实客户端已接受 Rust AUTH 票据，完成协议 210 大厅认证与 UDP v3 地址发现；端点 A 的大厅角色已得到动态验证。详见 AUTH_LOBBY_PROGRESS.md。
- 以目标样本指令、实际 MAC 验证后的请求和 AlphaBuild 参考交叉核对，恢复 10/12、10/21、10/10、12/6 的参数及结果格式；任务普通回执按 FIFO 消费，不能跳过未知任务后直接回复后续请求。
- 用户文件透明持久化已实现，并通过重启读回、title/UID 隔离测试；真实上传/重登尚待验证。新增源码、VA、证据置信度、实验路径见 SERVICE_INITIALIZATION.md。
- 实验 client-1791266444699 的真实诊断为 EXE_DOWNLOAD_PUBLISHER_FILE_FAILURE。目标必需标志和派发表确认 configs_s.info、release playlist 是当前必须补齐的两个公共文件；背包/大厅 UI 尚未恢复。
- 80/70 构造器与 0x7d0 结果对象已定位，语义暂推测为商城目录查询；没有套用旧版 getPlayerData 或玩家档案种子。

**公共配置与档案补充（后续阶段）**

- 目标自身 RSA-OAEP-SHA256 解包资源密钥成功，configs_s AES-192-CTR 与 AlphaBuild 同一算法/密钥；客户端内存验证配置版本、playlist 版本均为本地设置的 1，时间已写入正确 Unix 秒数。
- 公共配置致命错误已解除，8/1、8/2 返回目标明确定义的缺档案错误 800 后，客户端进入新的玩家数据请求。80/230 单结果结构已恢复，返回真实初始空账号数据后进入新玩家引导；截图仍有服务器忙提示。
- the.json 已通过真实上传及重登读回。8/3 真实公共档案上传格式、32 KiB zlib 快照及 CRC 均确认；Rust 持久化及重启测试通过，但最终动态实验未再次触发上传，实际公共档案上传—重登验收仍待完成。
- 最新证据、运行命令、已知 WineDbg 附加异常和下一步见 PUBLISHER_FILES.md。用户要求今后按验证阶段及时 Git commit。

**新玩家初始化后续补充**

- 真实公共档案两次 8/3 保存和下一次 8/1 重登已核对；独立 Python 验证回包 MAC、解密结果、磁盘和上传字节一致。对应证明在 public-profile-relogin-proof.json。
- 93/1 零余额、80/173 空附件、80/70→80/107 空测试目录已接入。只读状态探针测得目录状态 3、完成标志 7，最终截图无忙碌弹窗；不是物品或完整大厅恢复。
- 在 GameHub 私有 prefix 内用 C# Win32 ReadProcessMemory 实现无附加/注入的状态读取，解决此前 WineDbg 附加导致的异常。只读工具核对 EXE 哈希且只读取固定状态地址。
- 目标窗口的受控点击后出现 21/5；训练对局未实现。后续重点转向目标物品/角色表、private 档案及引导完成字段。最新完整记录见 ONBOARDING_PROGRESS.md。

**2026-10-07 游戏初始化验收补充（替代早期阶段状态）**

- Rust已完成登录/库存/本地房间/训练分配及DTLS、challenge/connect、stats、心跳、可靠消息引导。最新范围与限制见 [NATIVE_CHANNEL_PROGRESS.md](NATIVE_CHANNEL_PROGRESS.md)。
- `0x5855d0`（VA）注册dvar名称CRC seed、`0x585000`排序、`0x584d80`去重及`0x584ed0`累加经静态指令与只读资源提取交叉验证。mp_afghan计算CRC=`0x1f3a0a82`，与真实客户端初始化后的内存值一致，置信度高。工具与复现见 [NET_CONSTANTS_CRC.md](NET_CONSTANTS_CRC.md)。
- `client-1791342759140` 实测gamestateupdate1～6、连接状态9、服务端消息序号3；随后只读确认serverId=1。CS2需字面值`cod`的校验已定位在VA `0x445b74`～`0x445bb6`并修复下发值。
- 当前界面“等待玩家数据”，尚无出生、玩家移动、快照循环或AI。下一个目标为VA `0x46d170`的完整snapshot解析与VA `0x5a4230`的playerState布局；不得把初始化完成扩大为已经可玩。

**同日后续：快照、输入与场景显示**

- `client-1791345984530` 实际显示阿富汗地形与HUD。快照缓存和当前帧分别为flags8和flags0，状态10；总6798帧、251个入站事件无解析错误，native ACK最高6627。临时CA已恢复。当前实现仍是有界bootstrap，未完成武器/权威移动/AI。
- type19地图实体字符串只读提取给出第一条团队出生点 `-472 -800 -113.4`；目标内存确认坐标、视角高度60、viewLocked/viewLinked实体2047。反复发送基线帧仍黑；ACK后切换普通帧才观察到场景。静态依据与分阶段实测见 [SNAPSHOT_PROGRESS.md](SNAPSHOT_PROGRESS.md)。
- VA `0xc55f70` 的235项PS字段表已固定为协议事实fixture，独立提取脚本校验EXE身份。VA `0x5a1be0` 的usercmd已恢复三分支、额外11bit melee实体及第五个尾word，真实输入包完整解析成功；不执行客户端控制台文本或信任客户端预测位置。

**原生语音消费与资源证据**

- 本版VA `0x57b500`／`0x57b560`接受可靠字符串 `p`／`h` 的声音selector 1..16000；VA `0x444910`在selector<512时查CS3107+slot，其余减512查零起始NCS category5。静态指令与真实已初始化NCS布局互证，置信度高。
- VA `0x57b690`将 `cg_vo_prefix`送入 `0x4555c0`，VA `0x45567f`明确测试大写F；当前疾跑／呼吸句柄与Alpha地址不同。消费者与前缀字节条件置信度高；本版factiontable已实读确认阵营前缀；角色包到编号1／2的官方业务映射仍待恢复。
- 22张多人地图的初始化资源各含1494个声音常量、8个动态声音常量；竞技场诊断为1507／24。23份目录按当前各图CRC复算，包含8套男女／阵营角色语音，每套52个名字。名字与编号存在置信度高；安全屋／坠机已有实际声音通道、一次性事件及游戏音频证据，官方完整触发时机仍待恢复。
- 只读voice探针已有有效alias／原生声音通道采样；模式→开战的完整顺序及换弹听感获用户确认，正常地图准备启用模式、角色和结算语音。完整消费地址、捕获哈希、工具和验证边界见[原生语音](VOICE_PROGRESS.md)。


## 2026-10-13 Bot普通跳跃与路线变化

本地AlphaBuild的bot移动、随机目标及分级脱困已按源码核对，其A*仍为距离成本；3.3.3.3新增实现继续使用本版 `Motion` 的39单位普通跳跃和胶囊碰撞。候选助跑及跳跃先作真实运动验证，再按当前武器／战术倍率执行，路径快捷转向保留动作。区域随机偏好、近期行程避让和有移动意图时的分级重新规划已接。没有引入新的客户端地址。

高楼7666节点／4390条跳跃边、12条实际控制器落地，以及23图各60秒后台验证见 `research-output/bot-navigation-20261013/`；此轮 `native_client_used=false`。几何、权威运动和回归为高置信度；原生跳跃动画／观感未验收，翻越、80单位跳上及梯子尚未实现。细节见[BOT_NAVIGATION_PROGRESS.md](BOT_NAVIGATION_PROGRESS.md)。

**连杀奖励资源与协议依据**

- 本版标记副本只读捕获实际加载的29×33 `mp/coKillstreakStatsTable.csv`、44个商城kis条目及v233 schema。采集SHA-256为 `5079edf0cd80467a65b7c4143e5cbcea9d5b168fc4b253c280055842fef5dea6`；23项完整名字匹配，21项Pro没有本表对应行；稀疏奖励ID50不是物理行28。目录、字段和数值存在为高置信度，不证明所有条目可选或可用。
- 本版PE首选镜像VA `0x519820`／`0x519940`／`0x519980`分别读取PS+0x64、+0x68+4×slot、+0x7c+4×slot；235项PS字段表独立确认8／8／5位。VA `0x519860`选择下一个未达到目标或已配置最大目标。VA `0x4327d0`／`0x55d510`读取PS+0x560的五个2位动作类型及+0x574的五个11位动作武器；与参考服偏移不同。静态消费者和字段表互证，置信度高。
- 本版连接状态10的只读HUD采集SHA-256为 `2a2c0a0638927c95174d745da34a139372e99cc24165fd7ac76a9a11d71e3897`，确认四份连杀菜单、五个动作绑定及奖励表C0→C15／C1→C15／C1→C17图标查询。资源表达式为高置信度；未授奖或执行效果，不作玩法或视觉验收。
- 原始捕获与代码摘要留在 `research-output/killstreak-preparation/`；最小元数据由 `tools/prepare_killstreaks.py`复算。当前范围、未恢复规则和服务端接入方案见[连杀接入准备](KILLSTREAK_PREPARATION.md)。


### Bot普通跳跃的原生验收补充

2026-10-13固定版本、仅VM_READ的独立探针采样高楼／潜艇基地，CRC分别4261926685／3503358511；两图均有12名角色及本图结算，11只Bot持续移动，分别8／13次采到上升、下降与落地并匹配权威轨迹，AAR900:600／500:700与服务端一致。证据见bot-native-acceptance-20261013-pinned及bot-native-acceptance-20261013-subbase。首轮共享探针校验失败和潜艇基地选图自动开局误判保留，不计通过。新读取复用已核实的CG回放／PM类型字段，没有新增地址猜测。专用动作视觉与翻越／梯子仍未验收。
