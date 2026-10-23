# 文档索引

本仓库面向固定CODOL 3.3.3.3客户端和纯Rust服务端。文档描述当前状态、接口、运行方法与证据边界；不保存开发时间线。文件名中的 `PROGRESS`、`PREPARATION`、`PLAN` 和 `LOG` 保留为稳定链接，完成状态以正文为准。

初次运行先读[使用教程](USER_GUIDE.md)，独立部署与共享客户端接入读[服务器教程](SERVER_GUIDE.md)，已有本机环境的快捷入口见[本地游玩](LOCAL_PLAY.md)。了解整体能力读[功能状态](REFERENCE_ALIGNMENT.md)，开发前读[AGENT.md](../AGENT.md)和[模块结构](SERVER_MODULARIZATION_PLAN.md)。待完成实现及需要用户协助的体验统一见[复核清单](REVIEW_PENDING.md)。

代码实现、真实资源服务端验证、原生只读状态与画面／听感分别说明。证据文件保存在本机忽略目录，文档中无完整路径的证据名按专题查找；命令占位符见[运行手册](FOUNDATION.md)。

## 总览、运行与结构

- [使用教程](USER_GUIDE.md)
- [独立服务器使用教程](SERVER_GUIDE.md)
- [一键准备本地资源](RESOURCE_PREPARATION_GUIDE.md)
- [功能状态与验收边界](REFERENCE_ALIGNMENT.md)
- [待完成与体验复核](REVIEW_PENDING.md)
- [服务端重建范围与验证原则](REVERSE_ENGINEERING_PLAN.md)
- [权威玩法与验收范围](GAMEPLAY_PLAN.md)
- [工程基础与运行手册](FOUNDATION.md)
- [服务端模块结构与扩展约束](SERVER_MODULARIZATION_PLAN.md)
- [本地正常游玩](LOCAL_PLAY.md)
- [本地新玩家](LOCAL_PLAYER.md)
- [本地管理面板](ADMIN_PANEL.md)
- [原生客户端与 Rust 启动器接口](NATIVE_LAUNCHER.md)
- [地图加载与客户端启动](BACKEND_STARTUP.md)
- [原生客户端启动、隔离与恢复](CLIENT_STARTUP_RECOVERY.md)

## 样本、启动与协议

- [客户端样本与证据基线](CLIENT_BASELINE.md)
- [启动链与 dwauth.ini：静态发现](BOOTSTRAP_FINDINGS.md)
- [GameHub 客户端运行与 AUTH 诊断](GAMEHUB_DEBUG.md)
- [AlphaBuild 参考范围与本版适配](ALPHA_BUILD_REVIEW.md)
- [复用 AlphaBuild 逻辑，保持纯 Rust](ALPHA_RUST_PORT_PLAN.md)
- [2.6.9.7 加载文档对照：3.3.3.3 模型排查](LOADING_2697_REVIEW.md)
- [AUTH 与协议 210 大厅通道](AUTH_LOBBY_PROGRESS.md)
- [任务分发、时间与文件存储](SERVICE_INITIALIZATION.md)
- [游戏服分配与 UDP 加密通道](GAME_TRANSPORT_PROGRESS.md)
- [原生游戏通道、可靠命令与分片](NATIVE_CHANNEL_PROGRESS.md)
- [3.3.3.3 网络常量 CRC](NET_CONSTANTS_CRC.md)
- [快照、输入与复制历史](SNAPSHOT_PROGRESS.md)

## 账号、大厅与房间

- [公共配置与玩家初始化协议](PUBLISHER_FILES.md)
- [大厅初始化与只读客户端诊断](ONBOARDING_PROGRESS.md)
- [档案定义、私有档案与库存](PROFILE_INVENTORY_PROGRESS.md)
- [测试账号、等级与钱包](TEST_ACCOUNT_PROGRESS.md)
- [本地购买、币种与解锁边界](PURCHASE_PROGRESS.md)
- [人物与武器经验、结算与提示](PROGRESSION_PREPARATION.md)
- [本地房间与开局路由](ROOM_PROGRESS.md)
- [原生房间列表与多人入房路由](ROOM_BROWSER_PREPARATION.md)
- [房间 Bot 角色展示](ROOM_BOT_AVATARS.md)
- [原生选图、房间与结算](MAP_SELECTION_PROGRESS.md)
- [双客户端与多人联机验收](MULTICLIENT_ACCEPTANCE.md)
- [Ubuntu 远端双客户端验收](REMOTE_LINUX_ACCEPTANCE.md)
- [大厅、房间与游戏内文本聊天接入准备](CHAT_PREPARATION.md)

## 地图、模式与世界

- [团队竞技地图覆盖](MAP_COVERAGE.md)
- [原生地图入口与容量](NATIVE_MAP_ENTRY_RULES.md)
- [原生地图资源准备](MAP_RESOURCE_PREPARATION.md)
- [地图快速验收](FAST_MAP_ACCEPTANCE.md)
- [捕获出生点的碰撞筛选](CAPTURED_MAP_STARTS.md)
- [团队竞技开局与出生规则](MATCH_FLOW_AND_SPAWNS.md)
- [冲锋团队竞技原生接入](CHARGE_MODE.md)
- [原生出生朝向](SPAWN_ORIENTATION.md)
- [世界碰撞导出范围](WORLD_COLLISION_SCOPE.md)
- [原生玻璃破碎](NATIVE_GLASS.md)
- [高楼的玻璃与落差路径](HIGHRISE_TRAVERSAL.md)
- [地图油桶与可破坏车辆](MAP_PROPS.md)
- [原生小地图资源请求](RESOURCE_COMPASS_REQUESTS.md)
- [高楼与潜艇基地的小地图投影](MINIMAP_PROJECTION_PROGRESS.md)

## 玩家、装备与动作

- [玩家出生、时钟与装备同步](PLAYER_BOOTSTRAP_PROGRESS.md)
- [玩家背包、武器注册与物品所有权](PLAYER_LOADOUT_PROGRESS.md)
- [玩家角色外观与模型发布](PLAYER_APPEARANCE_PROGRESS.md)
- [第一人称持枪模型与动画绑定](VIEWMODEL_PROGRESS.md)
- [移动、输入与音频基础](GAMEPLAY_INPUT_PROGRESS.md)
- [原生飞扑机制（3.3.3.3）](DIVE_PREPARATION.md)
- [原生武器行为](WEAPON_NATIVE_PROGRESS.md)
- [通用武器配件与移动](WEAPON_ATTACHMENTS.md)
- [两/三/四发点射状态机](BURST_FIRE_PROGRESS.md)
- [Ranger两个扳机](RANGER_TRIGGER_PROGRESS.md)
- [第三人称持枪实体](HELD_WEAPONS_PROGRESS.md)
- [死亡武器掉落与拾取](WEAPON_DROPS_PREPARATION.md)

## Bot与战斗

- [Bot 战斗、生命周期与共享世界](BOT_PROGRESS.md)
- [Bot 跳跃、路线变化与脱困](BOT_NAVIGATION_PROGRESS.md)
- [Bot 避让与地面接触](BOT_GROUNDING_PROGRESS.md)
- [Bot阵营服装与原生角色组成](BOT_OUTFITS_PROGRESS.md)
- [Bot 第三人称换弹动作](BOT_RELOAD_PROGRESS.md)
- [Bot 单发狙击枪与第三人称动作](BOT_SNIPER_PROGRESS.md)
- [权威射击、弹着与战斗反馈](COMBAT_PROGRESS.md)
- [近战与投射物](MELEE_PROJECTILE_PROGRESS.md)

## 投掷物、技能与表现

- [投掷物](THROWABLES_PROGRESS.md)
- [被动技能](PASSIVE_SKILLS_PROGRESS.md)
- [拾荒者补给](SCAVENGER_PROGRESS.md)
- [闪光、震撼与技能抗性](TACTICAL_STATUS_PROGRESS.md)
- [毒气与战术面具](GAS_PROGRESS.md)
- [EMP与硬链接](EMP_PROGRESS.md)
- [个人与最终击杀回放](KILLCAM_PROGRESS.md)
- [前三名展示与结算名片](TOP_PLAYERS_PROGRESS.md)
- [原生开局播报与角色语音](VOICE_PROGRESS.md)

## 连杀与支援

- [连杀奖励覆盖状态](KILLSTREAK_COVERAGE.md)
- [连杀资源、背包与调用契约](KILLSTREAK_PREPARATION.md)
- [AlphaBuild 连杀机制适配](ALPHA_KILLSTREAK_MECHANISMS.md)
- [连杀公共机制分组](REWARD_GROUPS.md)
- [连杀运行时与原生验证](KILLSTREAK_IMPLEMENTATION_LOG.md)
- [UAV 与反侦察机](RADAR_REWARDS.md)
- [空中连杀支援](AIRSPACE_REWARDS.md)
- [空投奖励共用流程](AIRDROP_REWARDS.md)
- [选点连杀与轰炸](LOCATION_STRIKES.md)
- [连杀奖励部署设备](DEPLOYABLE_REWARDS.md)
- [SAM 防空奖励](SAM_REWARD.md)

## 剧情与资源采集

- [剧情模式与服务端接入](STORY_MODE_PREPARATION.md)
- [剧情测试解锁与货轮资源采集](STORY_RESOURCE_CAPTURE.md)

## 仓库根目录与样本

- [项目README](../README.md)
- [协作约束](../AGENT.md)
- [协议样本说明](../fixtures/protocol/README.md)
- [使用与分发边界](../LEGAL.md)
- [项目许可证](../LICENSE)

[实验性连杀资源与全图绑定](EXPERIMENTAL_REWARDS.md)说明资源准备、默认背包、逐项发奖规则及不保证范围。
