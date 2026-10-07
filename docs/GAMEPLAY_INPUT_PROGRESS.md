# 2026-10-07 移动、按键和音频接续

当前已从固定出生坐标推进到输入驱动的Rust移动，但用户实测仍有方向错乱、抖动和回拉，**移动尚未验收**。切枪崩溃已修并由用户确认。配装与全部物品见 [PLAYER_LOADOUT_PROGRESS.md](PLAYER_LOADOUT_PROGRESS.md)。

## 已接通

- 真实Afghan碰撞：type15的当前clipmap位于0x02dfb000，提取4245个player-solid凸刷体、55053个地形三角形。三角形索引包含partition.vertBlock*1024。
- Rust碰撞BVH、sweep、行走、滑墙、跨台阶、跳跃、蹲伏/趴伏；origin/velocity/ground/速度/重力/视高和jump字段写入235字段表对应位置。基础速度190、重力800、移动及身体比例1。
- generic float字段58/217和hint2尾按本版编码，非直接沿用旧C1布局。双枪与事件字段已经目标fixture校验。
- 时间用单调时钟推进，不再用已发送帧数代替实际时间。合理领先的输入排队等待，不丢弃按下/松开转换；远未来时间仍拒绝。
- 快照捕获增加服务器Motion与客户端predictedOrigin/time，供同一commandTime对照，不将客户端预测坐标直接当服务器位置。

## 当前方向/流畅度根因调查

用户反馈方向随转头变化，先是W右/S左/A前/D后，后变成斜向。目标AngleVectors、WalkMove和4bit输入映射均已核对，没有证据支持直接轮换WASD。

两次真实只读捕获确认 **渲染yaw恰好是玩家yaw的两倍**：

- `render-axis-live.log`：PS/current/client yaw约-8.7396°，render yaw342.5207°（即-17.4793°）。pitch没有翻倍。
- `render-axis-second.log`：PS yaw80.06836°，render yaw160.13672°。

视图轴来自CG+0xae510，屏幕投影0x43d960确认axis0是真实画面forward。相机角度CG+0xb2688，最终origin CG+0xae504。仍需定位重复加yaw的具体状态来源，不能把服务器移动方向乘2绕过。

此外离线回放12655个movement包，future输入几乎全部会重传，持续抖动不能主要归咎future丢包。对齐commandTime后静止位置中位差约1.9单位、运动约2.3单位，坡面几何/ground算法与客户端预测不一致正在修正。原实现box与客户端capsule差异、静态模型/动态实体碰撞缺失仍是边界。

## 音频

此前Wine日志持续 `dsound:get_mmdevenum ... 80040154`：游戏是32位，但prefix只注册64位MMDeviceEnumerator。已备份prefix注册表到 `research-output/audio-registry-backup-1791381753`，用引擎i386-windows/regsvr32注册同目录mmdevapi.dll，仅改项目prefix。

之后真实日志加载32位mmdevapi.dll及winecoreaudio.drv，原80040154消失；只读探针全局fade为1/1/0，macOS输出未静音。服务器也已写真实武器/动作事件。这些证明初始化阻塞解除，尚未独立录制或获得用户听音确认，不将“无错误日志”当作实际出声验收。

当前游戏prefix：`client-work/wine10-loadout-20261007`。只读探针编译prefix：`wine10-loadout-build-20261007`。
