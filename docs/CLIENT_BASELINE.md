# 客户端样本与证据基线

目标为本地 CODOL 3.3.3.3。`IIPSVersion.ini` 和资源清单指向该版本；主程序 PE 资源中的文件／产品版本为 `0.0.0.0`，不能单独确认版本。主程序为 Windows x86 PE32，离线启动器为 .NET PE32；PE 节名和保护组件文件的存在不足以证明实际保护或加载行为。

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

## 资源与运行环境

362 个 `.ifs` 的文件名、大小、nifs 文件头和 SHA-256 与本地参考清单核对。可识别列表的路径观察不能证明完整压缩块、最终 patch 覆盖或资源依赖闭包；实际地图必须独立初始化并复算 NCS CRC。资源盘点与可玩范围见[地图覆盖](MAP_COVERAGE.md)。

宿主机为 macOS arm64。服务端使用固定 Rust 工具链，客户端在项目隔离的 GameHub Wine/Proton 环境运行；构建、只读采集及恢复见[运行手册](FOUNDATION.md)和[客户端启动](CLIENT_STARTUP_RECOVERY.md)。

## 关键证据入口

| 范围 | 本版依据与验证入口 |
| --- | --- |
| AUTH／大厅 | 实际 JSON、独立票据向量、协议 210 的客户端证明；[AUTH](AUTH_LOBBY_PROGRESS.md) |
| 档案与物品 | public v233、private v19、6772 项物品表；[档案](PROFILE_INVENTORY_PROGRESS.md) |
| 游戏通道 | P-224、SHA1 派生、native 位池、gamestate 与快照；[传输](GAME_TRANSPORT_PROGRESS.md)、[快照](SNAPSHOT_PROGRESS.md) |
| 飞扑 | 本版起扑、移动／武器状态、落地事件与原生平地操作；[飞扑](DIVE_PREPARATION.md) |
| 掉枪 | ITEM3 使用 `weapon + 900×modelSlot`，原生 cursorHint 字段；[掉落](WEAPON_DROPS_PREPARATION.md) |
| 房间装配 | type18 的 649 位装配、前端 UID 缓存和原生双队展示；[房间角色](ROOM_BOT_AVATARS.md) |
| 等级 | 三套本版经验曲线、事务与可靠更新、原生重登／武器等级；[等级](PROGRESSION_PREPARATION.md) |
| 连杀 | 原生奖励表、库存／进度、效果控制器和各项实机边界；[连杀](KILLSTREAK_COVERAGE.md) |
| 多人 | 双身份同房、共享战斗、真人同步及结算；[本机](MULTICLIENT_ACCEPTANCE.md)、[远端 Linux](REMOTE_LINUX_ACCEPTANCE.md) |
| 聊天 | 公共 102/3、房间 type2 及拒绝路径；[聊天研究](CHAT_PREPARATION.md) |

地址必须区分 VA、RVA 和文件偏移。静态消费者、合成向量、真实资源服务端验证、只读客户端状态和画面／听感各自说明证据强度；不能把参考版本或准备数据当成本版玩法验收。原始包、完整反编译、资源和账号材料保存在忽略目录。
