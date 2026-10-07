# 玩家出生配置、发包时钟与装备

2026-10-07，接续 [快照进展](SNAPSHOT_PROGRESS.md)。已显示场景/HUD；本轮首先解决此前约33秒才有一包输入的问题。

## 发包时钟根因与修复

只读对照 `client-1791346873591/client-state-1791346931462.json` ～ `client-state-1791346936492.json`：maxPackets=30，但真实约5秒内clientRealtime仅34258→34263、commandNumber17→22。`client-state-1791347329127.json` 进一步显示fixedTime=0，frameScale/scriptTimeScale原始bits均为27346044，解为接近零的浮点值。

目标VA `0x460210` 判定发包间隔：`1000 / dvar969.current` 与 clientRealtime−lastPacketTime 比较，当前门槛33毫秒。`0x467a00` 增加clientRealtime；`0x59d500` 将缩放后的帧时长下限设为1毫秒；`0x59d840` 同时按这个近零倍率把实际帧频限至1。因此约33秒才达到发包门槛，并非UDP或Mac网卡吞包。

VA `0x579f30` 从**configstring6415（0x190f）**取得文本，使用 `0xc6b49c` 的 `%i %i %g %g` 解析慢动作起点、时长、起始/结束倍率，但不检查sscanf返回值。之前未下发这一字段，局部变量残留进入 `0x59d2f0` 的时间倍率过渡，再由 `0x59d490` 参与帧时间缩放。只读type表确认6415类别为255，可以普通configstring覆盖。

Rust gamestate现在下发 `6415 = "0 0 1 1"`。没有修改客户端函数、内存或本地dvar文件。

`client-1791347562580` 验证：frameScale bits=1065353216（1.0）；采集之间约2秒，clientRealtime 75752→77793→79807，commandNumber2872→2999→3131。超过3400条native包无解析错误，间隔中位数约40ms，恢复正常数量级。准确速率随前台焦点、帧频及调度变化，仍须持续观察。

这也纠正快照文档中“输入较稀疏，原因待查”的旧边界。物理移动仍未实现；输入频率恢复不能代替权威位置积分、碰撞或装备执行。

## 装备接续

准备从同一地图的NCS category13恢复武器ID，不能使用商城item ID直接填入playerState。当前资源中基础 `m4_mp` 对应索引163，`m9_mp`为180；这些值必须从排序/去重后的实际资源导出，而不是跨版本硬编码。

目标 `0x5a18e0` 每个输入包都从当前playerState生成默认命令：PS+0x340、+0x32c、+0x344分别提供当前武器、offhand、alt；PS+0x10的0x1000标志还会将默认melee实体设为2047。新增武器/玩家标志时必须同步输入base。具体下发字段、后续实测结果在实现验收后补充。
