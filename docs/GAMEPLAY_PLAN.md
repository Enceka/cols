# 权威玩法与验收范围

Rust 服务端负责移动、碰撞、装备、战斗、死亡、重生、计分和胜负。客户端负责原生预测和显示，输入中的预测位置仅用于对照，不作为权威坐标。总体实现范围见[功能状态](REFERENCE_ALIGNMENT.md)。

| 系统 | 当前实现与专题 |
| --- | --- |
| 原生通道 | 独立方向头、XOR、可靠命令、压缩、索引式分片；[通道](NATIVE_CHANNEL_PROGRESS.md) |
| 出生与装备 | 按地图与模式绑定点位、朝向、真实配装和模型；[出生配置](PLAYER_BOOTSTRAP_PROGRESS.md)、[背包](PLAYER_LOADOUT_PROGRESS.md) |
| 移动 | 胶囊碰撞、滑墙、台阶、跳跃、姿态、冲刺及飞扑；[移动](GAMEPLAY_INPUT_PROGRESS.md)、[飞扑](DIVE_PREPARATION.md) |
| 战斗 | 距离伤害、部位判定、敌我关系、弹着、命中反馈、投掷物与地图物件；[战斗](COMBAT_PROGRESS.md) |
| Bot | 导航、交战、动作、死亡重生及房间补位；[Bot](BOT_PROGRESS.md) |
| 比赛 | 准备／宽限、战术动态出生、冲锋基地、最终回放／前三名／结算／返房；[比赛流程](MATCH_FLOW_AND_SPAWNS.md) |
| 多人 | 房间共享世界，独立连接复制；真人可见、移动／交火、一致结算和再次开局已验收；[多人](MULTICLIENT_ACCEPTANCE.md) |
| 经验 | 显式本地策略下完成 TDM 对局的权威奖励与持久化；[等级](PROGRESSION_PREPARATION.md) |
| 连杀 | 循环门槛、稳定库存与部分效果；各项可用性见[覆盖表](KILLSTREAK_COVERAGE.md) |
| 剧情 | 关卡测试解锁、独立准入和首关资源采集；任务／NPC 玩法待接；[剧情](STORY_MODE_PREPARATION.md) |

单人 Bot 模式的验收不能直接推广至共享多人。玩法改动须分别检查输入去重、实际资源、权威状态、复制与客户端消费；需要视觉／听感的项目见[待复核清单](REVIEW_PENDING.md)。正常准备维持 7500 分／10 分钟，缩短对局或聚集敌人的诊断配置保持隔离。
