# 原生武器行为接续

2026-10-09。用户最新范围：6v6（玩家+5友军，对6敌军）、TDM、尽量使用原生难度和地图选择；插入优先修复自由形态切换、半自动预测重复开火、霰弹枪拉栓、伤害和盲射散布。此前爆头图标、爆头计数和死亡后换背包已由用户验收。

## 已实现但仍需最新实机验收

- 探针导出原武器引用的6个alternate-only定义及其配件属性。它们没有商城物品，不作为可单独持有的枪。
- 服务端合成alternate自己的属性，注册其原生命名/编号，维护原武器、副武器及各自alternate。原生共享弹匣标记+0xdad使两形态共享clip key和备弹。
- 自由原生变形：drop state5/anim19/+0x754，然后raise state2/anim20/+0xd28。下发PS+0x344配对模型武器。**0x5a18e0的实际输入顺序为primary、paired、offhand；以前接续文档和首次修复误写为primary、offhand、paired，已修正。**最新`Equipment::input_weapons`统一该顺序。
- 半自动同步PS+0x284（field87，3bits）的本次扳机射击次数。0x424c70还在长按耗尽次数时将weaponTime钉在1ms；当前服务端同样保留该状态。用户此前观察到第二枪动画、预测扣弹、松手回弹，尚不能把此前单元测试通过算作实机修复。
- idle按0x421fe0回到animation0，替代之前的animation1。
- +0xd74拉栓枪：fire冷却后state7，hip anim4/ADS anim7，+0x700计时，+0x708退壳标记、event40/41。SPAS/自由主形态原始192ms+550ms，实际配件仍参与合成。
- 散布：所有子弹均应用散布，单弹丸不再强制精确射线。从合成定义读取+0x810..0x83c站/蹲/趴最小最大、恢复、开火/转向/移动增量，ADS散布+0xac0。按0x422aa0/0x425840更新PS+0x350的0..255散布量并下发field6。三维散布采用视线垂直平面；确定性随机种子仍是服务端实现，未认证为客户端原始随机序列。
- M200（cheytac_mp）快瞄握把22208/cqc_quickdrawcheytac实际记录是kind9、op8，在+0xad0/+0xad4各加原值的-12%；400ms变352ms，合成器已按0x42af20重新计算+0xbac/+0xbb0。新增回归覆盖实际记录；不是完全没效果，也不擅自改成其它百分比。
- 霰弹枪射程按原生class4/+0xbe0末段距离，默认扩散取实际最小值，之后由散布状态更新。每颗实际命中的弹丸按距离和部位计算并累加；近距离可能一枪击杀。日志新增总damage、pellets_hit和distance。用户仍对一枪秒有疑问，尚无官方对局伤害对照，不任意压低表值。
- 左下比分暂用7500上限修复score_limit=0的缩放。完整TDM计分与结束规则尚待接线。

## 验证及当前未完成工作

`research-output/native-final-check.log`：260项Rust测试、30项Python、fmt/Clippy通过；真实回放诊断另有1项ignored。动态数据留忽略目录，`research-output/native-weapons-state.json`已引用新含alternate的arsenal采集，不要用旧准备中的arsenal路径测试第二形态。

最后一次用户实测的是`client-1791500468048`，仍有第二形态待机隐藏、半自动预测重复射击。该次运行**没有**最新的paired输入槽修正、1ms等待、idle0以及全武器动态散布。下一次运行必须使用新编译binary；这些修复尚未获用户验收。

原始反编译入口：`native-spread-verified.c`（0x422aa0/0x425840/0x55bc50）、`native-fire-release.c`（0x4248c0/0x424c70）、`native-input-base.c`（0x5a18e0/0x421fe0）、`alt-switch*.c`、`weapon-fire-timing.c`。中间若干Containing导出始于函数内部，不能单凭它们的错误调用约定判断字段。

6v6的未提交草稿在`crates/server/src/game_match.rs`，目前未接入lib/Connection；多人射线草稿在忽略目录`research-output/match-combat-wip.rs`。快照已扩为players/baselines数组、尸体槽19..26、obituary客户端0..18；仍保持训练路径以便武器验收。正式推进时需要接入match状态、出生点、多人伤害、比分和结束规则，并替换训练入口。地图选择解析/资源目录已存在，但CLI尚未传入catalog，第二地图尚未采集，不能借Afghan CRC冒充Killhouse。
