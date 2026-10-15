# 移动、输入与音频基础

Rust 权威移动由已校验 usercmd 驱动，使用本图真实碰撞；预测 origin 仅作同 commandTime 比对。配装和武器见[背包](PLAYER_LOADOUT_PROGRESS.md)，疾跑、技能和飞扑见[被动技能](PASSIVE_SKILLS_PROGRESS.md)与[飞扑](DIVE_PREPARATION.md)。

## 移动和碰撞

- 世界使用 BVH、胶囊 sweep、滑墙、跨台阶、跳跃、蹲伏／趴伏，按实际接触处理地面和重力。
- 本版基础速度 190、重力 800；配件和技能在各自规则内应用倍率，不修改客户端预测来源。
- origin、velocity、ground、速度、重力、视高和 jump 写入本版 235 项字段表；generic float 和 hint2 尾按本版布局编码。
- 单调时钟处理输入，合理领先命令排队，保留按下／松开边沿；死亡、冻结与重生清理待执行动作。
- 胶囊碰撞使用当前完整位置和速度积分，落地不重复推进；身体旋转参考使用未绑定实体 2047。

同命令实采已验证正常静止位置和移动误差，平地飞扑有独立原生对照。特殊地形、复杂物件边缘的足部接触及完整跨越动作仍需专项验收。BSP 导出边界见[世界碰撞](WORLD_COLLISION_SCOPE.md)，Bot 地面约束见[Bot 接地](BOT_GROUNDING_PROGRESS.md)。

## 输入和武器注册

同账号重连重放既有组合名称和编号，按父枪、原生替代模式、副枪顺序注册。不能每次从相同编号重新排序而与客户端缓存冲突；服务端重启后客户端也需要按启动流程恢复相同资源基线。

commandTime、输入 base、ADS 默认按钮与 ACK 的关系见[出生同步](PLAYER_BOOTSTRAP_PROGRESS.md)。普通游戏输入与自动化窗口操作均校验目标实验 PID／归属和前台；焦点失败不发送输入。

## 音频

客户端兼容环境需要有效的 32 位 mmdevapi／winecoreaudio 注册和正常输出设备。资源加载与无错误日志不能独自证明出声。基础游戏声音、开场和角色播报已有原生听感证据；逐项命中、特殊武器和空间衰减仍按[语音专题](VOICE_PROGRESS.md)与[复核清单](REVIEW_PENDING.md)验收。
