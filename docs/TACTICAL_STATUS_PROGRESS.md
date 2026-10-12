# 闪光、震撼与技能抗性

纯Rust支持背包中的闪光60004／原生武器89和震撼60007／原生武器57，沿用Q按钮0x10000、选定槽PS+0x338、独立数量及原生offhand阶段。闪光拉环600ms、投出699ms、延迟150ms、引信1200ms；震撼分别为200／300／100／1000ms，均不在持有时消耗引信。数量当前仍按背包双弹量标志给1／2；原生默认出生数量与额外数量的完整生产规则未恢复，不能由WeaponDef capacity2单独推出。

## 原生资源与传输

`fixtures/protocol/tactical-status-3.3.3.3.json`由安全屋只读`tactical-assets-1791721644930.json`派生。NCS category7有8个继承名字，闪光`flashbang_mp`为index5，震撼`concussion_grenade_mp`为index1。Named shock参数数组16项、stride0x268，实际原文为`shock/NAME.shock`；直接使用本版继承绑定，不向gamestate重复广播这些已类型化名字。

闪光原生screen_type1，白屏／截图衰减常数3500／1000ms；震撼screen_type0、look_control与movement启用、75度每秒的俯仰／水平限制及0.1鼠标敏感度。实际参数和原文哈希保留在fixture，不能把视觉衰减常数当成命中状态的生产持续时间。

PS field105／103／104对应index／begin／duration；104属于hint2尾部，按严格早于ordinary lastChanged的本版规则传输。完整帧与delta均从零PS基线编码，duration为16bit时长；回放只重基非零begin。到期、死亡和重生保留最后有效Named index，以begin0／duration0明确触发原生reset；index0可能使用客户端本地时间覆盖，不能用于保证取消。震撼的PM flags0x8000由服务端管理，在移动与持枪近战更新中保留，到期明确清除；本版原生移动消费者使用0.4倍率，modAsymRobot位另选0.75。

闪光世界事件为EV92／eType114，震撼为EV_CUSTOM_EXPLODE93／eType115，并携带真实爆发起始时间；烟雾专用flags0x40不会留在震撼实体。爆炸FX与受害者状态是两个独立路径。

## 服务端规则及边界

本版客户端没有提供已确认的服务器生产脚本。当前持续时间按工作区Alpha／LAN参考脚本移植，来源和哈希明确记入fixture；这不等于本版官方生产规则已核实。闪光参考值为距离暴露×朝向暴露×6000ms，朝向低于0.5取0.5、高于0.8取1；原始时长低于250ms不产生状态。震撼为2000＋4000×距离暴露ms。当前距离以脚底到爆发点、朝向以视点前向和爆发方向的点积估计，世界遮挡使用单条眼点射线；完整身体／骨骼／部分遮挡及原始amount_distance／amount_angle消费者未恢复。

实际受害者的modFlashProtectionScale／modStunProtectionScale进入时长计算。战术面具60137的两项本版聚合值均为0.1；这里只称参考脚本倍率移植，官方倍率的生产应用点仍未恢复。同帧取较长状态，不同帧使用原生单个shader槽，完整异类状态叠加尚未恢复。队友免伤免状态，玩家可受自己投掷物影响；闪光／震撼的WeaponDef范围512和伤害1单独进入原有判伤。毒气的两套原生Named预设、周期判伤与面具免疫见 `GAS_PROGRESS.md`。震动手柄、EMP和硬链接抗性消费者仍未实现。

Bot闪光期间不获取／使用当前可见目标，保留最后已知位置；震撼期间按本版控制倍率减慢实际移动、限制转向，结束后恢复。该感知行为是当前服务端AI策略，不声称恢复了官方Bot全部状态行为。Bot死亡／重生和目标记忆失效分开处理，目标死亡不会提前解除自身状态。

## 验证

`tactical-final-check.log`：397项Rust、52项Python、fmt／Clippy通过。覆盖独立PS表解码、hint2边界／清零、Named取消、事件复用清除烟雾flags、回放重基、抗性／朝向／世界遮挡、队友过滤、死亡重置及Bot实际移动／转向与恢复。`prepare_tactical.py`从实际捕获重新派生的结构与fixture一致。

`tactical-flash-no-mask-proof.json`和`tactical-flash-mask-proof.json`分别记录隔离账号两组400帧近距离Q投出。无面具原生index5／duration3824ms，106帧状态、209帧Named取消；带面具385ms，12帧状态、301帧取消。两者客户端CG缓存均读到对应参数指针／begin／duration，投掷物1→0，玩家99／100生命；无面具47帧读到真实`viewmodel_m84`，世界`explosions/flashbang`亦生成。两组几何／朝向并非逐位相同，倍率按服务端日志与状态参数确认，不把时间比值单独当作官方公式证明。

`tactical-concussion-no-mask-proof.json`记录另一组400帧近距离Q投出：原生index1／duration4377ms，125帧状态（123帧同时读到PM flags0x8000与正确CG缓存），203帧Named取消，取消片段震撼位全部清零；世界FX和22帧飞行实体生成。单次W输入在片段中另行记录，完整原生预测／服务器坐标对照仍需专门覆盖，不能只因状态位存在就当作全部运动验收。

`tactical-concussion-mask-proof.json`记录面具对照：原生index1／duration444ms，短状态与随后Named取消均实采，结束后震撼位清除；与无面具4377ms的几何并非逐位相同，倍率的公式来源与边界保持上述记录。

原生普通／最终回放、状态叠加、更多遮挡边界、音效听感和完整画面仍需自主复核。不能仅以参数缓存装载或能力位发布声明全部效果验收。
