# 高楼的玻璃与落差路径

`mp_highrise`已进入正常目录。本图CRC4261926685、实体、BSP世界碰撞、10／9个有效开始点、武器、角色及真实1024×1024小地图材质均有对应捕获。正常准备为`research-output/tdm-highrise-state.json`，保持7500分／600000ms。原生准入证据为`highrise-map-admission/admission.json`及其引用报告。

本图169块可破碎玻璃使用真实对象和碰撞编号；服务器有效射击／近战改变相应碰撞并同步原生破碎状态，见[原生玻璃](NATIVE_GLASS.md)。导航规划可以经过可破碎玻璃，实际移动必须等待正常命中移除相应碰撞；永久WINDOW刷子保持阻挡。

行走图额外接入有方向的落差边。候选只能向下18..128单位，水平距离24..72单位；用同一站立胶囊、运动、重力及Bot停止行为模拟，必须真正落到目标的捕获地面，水平误差不超过18、垂直误差不超过1单位。没有地面、永久墙体、过深／过远的落差或反向攀登均拒绝。行走连接、近点连接和快捷转向继续要求地面支撑，不能以一条空中直线代替跨越能力。

`highrise-glass-acceptance/server-drops-60.json`记录60秒模拟的11个Bot移动、14块玻璃破碎、5次击杀、4次重生及前三名地面场景生成。`highrise-native-acceptance-retry/native.json`的原生短局162.22秒确认12名角色／动画树、真实小地图、同生命周期移动142.52单位和12行结算；AAR200:300、Allies获胜与服务端一致，结算后成功验证原生箭头往返且保留房间。自动流程0截图，完整观感仍需专项复核。

真实实体另含124对`node_negotiation_begin`／`node_negotiation_end`，`highrise-traversal-resource-evidence.json`保留原始实体SHA及配对：35条`jumpdown_40`、31条`mantle_on_40`、24条`jumpdown_96`、19条`jump_up_80`，以及其它跳上／翻越和一条梯子连接。`highrise-traversal-physics.json`记录553次隔离候选运动，不改正常对局；它不代表已接入全部原生AI动作。已接物理验证的普通跳跃、助跑和路线变化，详见[BOT_NAVIGATION_PROGRESS.md](BOT_NAVIGATION_PROGRESS.md)；普通跳跃不等同于原生翻越、80单位跳上或梯子动作，专用动作动画仍待接入。

正常会话已恢复，6772库存与原始资源保留。20图后台批量均通过；地图完整视觉／听感和长时间稳定性另行复核，不能把本图通过写为26图全部通过。
