# AlphaBuild 连杀机制适配

运行时保持纯 Rust，AlphaBuild 用于参考完整控制流程，本版 3.3.3.3 的目录、资源和协议独立适配。参考目录为工作区 `CODOL-AlphaBuild/CODOL_AlphaBuild_LAN 服务器端/CODOL_Server/dedicated`；35 个相关 GSC 文件的路径、函数入口和完整 SHA 保存在忽略的 `alpha-killstreak-mechanisms.json`，原脚本不复制进 Rust 仓库。

## 公共调用和库存

参考 `_killstreaks.gsc::killstreakUsePressed` 按稳定 `kID/lifeId` 找到奖励，检查存活、地面、EMP、远控、位置选择、炮塔和武器状态，再调用效果函数；成功后复核该奖励仍归调用者所有，最后消费实例。效果失败、选点取消和权限撤销保留库存。HUD 槽位变化不能使延迟回调消费另一个奖励。

`game_reward_inventory.rs` 实现稳定实例、每生命自然奖励去重、预留、精确消费、取消和生命周期撤销。现有普通奖励处理器共用该库存；跨生命再次赚到相同奖励生成独立实例，每次成功调用只消费一份。切换装备预览不会删除已赚库存，死亡取消未完成的调用权限并保留库存，对局结束清理。当前 HUD 仍按已选三项奖励投影，未选中的库存保留，选回后可用；空投新奖励的槽位投影仍待接入。尚未完整实现的选点、标记弹、拾取和远控控制器需要调用同一实例接口；它们不能因该公共模块存在而被列为完成。

25 杀核弹目前仍走独立的固定额外槽及单次倒计时状态；王牌、Pro 与模式奖励的入口和原生表现继续单独核对。

## 按机制组织效果控制器

| 机制 | AlphaBuild 参考入口 | 本版接入内容 |
| --- | --- | --- |
| 公共所有权与消费 | `_killstreaks::killstreakUsePressed`、`codolRewardByID` | 稳定实例、权限检查、效果成功后消费、失败或生命周期变化取消 |
| 空投与拾取 | `_airdrop::tryUseAirdrop`、`tryUseAirdropSamTurret` | 标记弹、运送节点、箱体碰撞、持有拾取、随机或指定内容；普通/多箱/哨戒/SAM 共用控制器 |
| 位置选择与空袭 | `_airstrike`、`_mortar`、`_harrier` | 选点/取消、地图空域、弹道与范围伤害、实体期限；精确空袭/迫击炮/隐形轰炸共用选点流程 |
| 远控与炮手 | `_remotemissile::tryUsePredatorMissile`、`_ac130`、`_rcbomb` | 进入/退出远控、身体保存、相机、输入路由、武器切换、死亡和断线恢复 |
| 攻击飞行器 | `_helicopter`、`_littlebird`、`_harrier` | 真实 authored 节点、目标与出生保护、装甲伤害、诱饵弹、受损/坠毁/离场 |
| 部署设备 | `_autosentry::spawnTurret`、`sentry_setOwner` | 放置预览、拾取搬运、炮塔方向、动画/过热/声音、归属和死亡处理 |

C1 的参数和资源名称必须与本版捕获比对；C/C++ 的旧字段表、指针地址、武器号和偏移不进入本版线格式。

## 目录与路线边界

AlphaBuild 通过其目录列 22 关联 Pro 与基础奖励。本版捕获的 `mp/coKillstreakStatsTable.csv` 为 33 列，29 个物理行，该列没有 Pro 关联行；其 ID 23 已是 `juggernaut`，不能套用 C1 的 Pro 行 ID 23..43，也不能删商城名称后缀猜对应关系。

AlphaBuild 的 `_uav` 使用 minimap 中心、隐藏旋转 rig、较远的三维偏移和末段离场，默认包含绕圈。这提供生命周期、敌方可见性、伤害和离场机制参考，不覆盖用户指定的高空平直航迹要求。当前水平直线的高度与速度为明确兼容参数；原版精确参数及当前路线的原生验收仍需完成。
