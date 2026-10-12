# 高楼的Bot跨越缺口

`mp_highrise`的资源准备已有本图CRC4261926685、实体、BSP世界碰撞、10／9个有效开始点、武器、角色及真实1024×1024小地图材质。资源证明见`compass-candidate-live/result.json`及`compass-two-map-acceptance`。该图尚未通过玩法验收，不在正常目录。

`compass-two-map-acceptance/acceptance/server.json`的60秒后台模拟和`highrise-server-120.json`的120秒定向模拟均记录11个Bot移动，但0次击杀和重生。两端开始点所在行走组件与中央区域未连通；诊断中的细网格也未得到共享路径，不能把增加模拟时间或移动统计作为交战通过。完整视觉和原生对局仍未验收。

本图真实实体包含124对`node_negotiation_begin`／`node_negotiation_end`。`highrise-traversal-resource-evidence.json`记录实体捕获SHA、目标配对、起止位置及动画脚本，其中35条`jumpdown_40`、31条`mantle_on_40`、24条`jumpdown_96`、19条`jump_up_80`，还包含其它跳上／翻越及一条梯子连接。两端开始区附近存在40单位跳下／翻越记录。这是恢复跨越路径的资源依据，不代表已实现原生AI路径或动作。

本图169块可破碎玻璃已有真实对象与碰撞编号、服务器射击破碎及原生快照同步，见[原生玻璃](NATIVE_GLASS.md)。导航规划可经过可破碎玻璃，实际移动必须等待真实命中移除相应碰撞。`highrise-glass-acceptance/server-60.json`仍记录11个Bot移动、0次击杀；服务器日志记录一次真实射击破碎，但这没有证明开始区与中央区域连通。现有19张正常地图的快速回归全部通过，高楼仍不在正常目录。

当前`BotController::move_toward`只执行行走／跑动／冲刺，不执行跳跃或改变姿态；`Navigation`只生成有站立胶囊及地面支撑的行走连接。高楼后续需对照捕获的negotiation位置验证窗台、落差及跨越轨迹，再接入相应路径、服务器运动和动画同步。保持真实碰撞、伤害、移动及击杀检查；不通过移除永久阻挡体、把两队开始点搬到中央、传送Bot或弱化交战要求来宣称通过。
