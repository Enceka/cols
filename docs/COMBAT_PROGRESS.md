# 权威射击、弹着与战斗反馈

训练靶和团队竞技均由 Rust 校验射击、判定世界遮挡、应用伤害及同步结果。Bot／真人敌我关系按当前队伍处理；无效输入、空弹匣、菜单、换弹和切枪期间不生成额外伤害。共享多人核心交火与重生已有[原生验收](MULTICLIENT_ACCEPTANCE.md)。

## 射线与伤害

普通子弹从眼位沿实际输入视角发射，长度 8192，使用本版 shot mask `0x0280e831`。移动与射击各用自己的碰撞掩码。组合 WeaponDef 提供六档伤害与距离，段间插值、远端使用末段，玩家和 Bot 使用同一数据来源。

目标粗盒用于候选筛选，骨骼 OBB 用于实际部位判定，部位倍率来自本版采集。队友可以挡弹但不受普通友伤；出生保护和无效命中不产生成功反馈。完整穿透和特殊投射物边界见[原生武器](WEAPON_NATIVE_PROGRESS.md)及[近战](MELEE_PROJECTILE_PROGRESS.md)。

训练 GENERAL 靶独立于地图物件：模型边界、scale、100 生命和 3 秒重生由诊断准备绑定。地图油桶／车辆有独立燃烧、爆炸、残骸和连锁，见[地图物件](MAP_PROPS.md)。

## 世界弹着

普通世界弹着使用 ET_EVENTS22＋EV_BULLET_HIT74，即 eType96；携带命中位置、法向、实际武器、物理材质和世界实体2046。事件有独立身份和有限存活，不因重发重复播放。

GENERAL／event 的 X、Y 字段顺序与 PLAYER 不同，必须各自按本版表编码。Brush 侧面和轴向材质分别提取，顺序映射为 `+X,-X,+Y,-Y,+Z,-Z`。WeaponDef+0xd0c 选择实际 impact FX 行，不能统一发送 surface0。

sky4、noimpact16、nomarks32、nodraw128 保守抑制弹着事件，几何仍挡弹。仅抑制 decal 而保留声音／粒子的完整标志尚未恢复。

## HUD、声音与死亡定位

普通／致命反馈使用本图 `feedback_resources`，白色／红色分别按 1000／600ms 淡出。声槽以实际 sound bank 注册 `MP_hit_alert`，使用可靠命令 `p <relative-slot>`；资源加载与独立听感分别验收。

命中反馈属于 archival HUD（PS+0x1ed0），队友死亡定位属于 current HUD（PS+0xa78）；线上顺序先 archival 后 current。死亡标识只展示同队队友，重生或到期清除，视野外不画边缘箭头。归档和个人／最终回放见[回放](KILLCAM_PROGRESS.md)。

协议向量见 `fixtures/protocol/damage-feedback-resources-3.3.3.3.json` 和 `death-waypoint-hud-3.3.3.3.json`。回归覆盖两阵营、遮挡、出生保护、致命优先、位池、替换／清理和时钟回绕。阿富汗原生红色击杀及再次白色命中已有证据；其它地图和独立提示音按[复核清单](REVIEW_PENDING.md)继续覆盖。
