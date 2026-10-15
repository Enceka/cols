# Bot 战斗、生命周期与共享世界

Bot 使用本版 PLAYER／client、实际角色包、组合武器和 human 动画树，在服务端权威世界中执行导航、交战、伤害、死亡和重生。它没有账号档案或游戏 socket；昵称统一为 `Bot n`。训练用单敌人和正常团队 Bot 使用各自准备，不能把诊断参数当作全部 Bot 规则。

## 当前能力

| 范围 | 行为与专题 |
| --- | --- |
| 导航 | 真实碰撞、路线偏好、跳跃与脱困；[导航](BOT_NAVIGATION_PROGRESS.md) |
| 接地 | 胶囊接触、避让与捷径支撑检查；[地面约束](BOT_GROUNDING_PROGRESS.md) |
| 射击 | 视线、敌我关系、实际射速／伤害、距离曲线及弹药／换弹；[战斗](COMBAT_PROGRESS.md) |
| 外观 | 阵营角色、原生组合枪、回放衣袖／手套；[服装](BOT_OUTFITS_PROGRESS.md)、[持枪](HELD_WEAPONS_PROGRESS.md) |
| 动作 | 移动腿部与换弹上身分离，按武器重新绑定；[换弹](BOT_RELOAD_PROGRESS.md)、[狙击枪](BOT_SNIPER_PROGRESS.md) |
| 死亡 | 伤害事件、尸体、计分、动态出生、回血与保护；[比赛流程](MATCH_FLOW_AND_SPAWNS.md) |
| 房间 | 原生角色装配、名单、阵营与真人替换；[房间展示](ROOM_BOT_AVATARS.md)、[多人](MULTICLIENT_ACCEPTANCE.md) |

普通子弹使用真实六段距离伤害；部位判定以采集骨骼 OBB 为依据。粗盒只作候选门槛，未命中实际区域的子弹继续检查世界。爆头同步原生图标及 headshots 计数，不能用直接调用扣血替代射线验证。

死亡序号用于单次计分／经验去重，尸体复制 donor 外观，复活递增原生 spawn_count 并重置本条命状态。共享多人中一局只推进一份 Bot 状态，各接收者有独立实体映射与复制历史。

复杂物件边缘足部贴地、专用跳跃／翻越／梯子、更多枪械动作和长时间路径稳定性仍待完成或复核。原生模型加载、状态 ID、完整动作和听感分别记录，见[待复核清单](REVIEW_PENDING.md)。
