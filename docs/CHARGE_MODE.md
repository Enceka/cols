# 冲锋团队竞技原生接入

本版冲锋使用 `cha`、模式 NUMBER 15；战术使用 `war`、NUMBER 1，`tdm` 是战术别名。房间、资源目录及 Bot 外观按 `(map, mode)` 绑定，同图两种模式可以同时存在。

默认 `research-output/native-tdm-map-catalog.json` 含23份战术准备及核弹镇1份冲锋准备。原生入口为“玩 → 多人竞技 → 创建房间 → 冲锋团队竞技”，用地图预览箭头选核弹镇，再点准备，11名 Bot 加入组成12人房间。其它冲锋地图只保留本版身份，不视为已准备或已验收。初选地图受原生保留状态影响，服务端不改本版开启标志。

核弹镇准备为 `research-output/charge-mode-20261015/mp_nuked-ready-state.json`，正常7500分／600000ms。资源来自实际进入 `cha/mp_nuked` 的只读捕获：NCS初始化完成，实际CRC为1765105316；碰撞、武器、角色、服饰及真实小地图材质均核对本图身份。初始化探测元数据不参与正常游玩，也不作为资源准备证明。

本图提供9／9个冲锋开局点，站立碰撞筛选后的基地复活池保留8／9个点。开局和15秒宽限使用本队开局阵型，之后从本队 `mp_cha_spawn_axis`／`mp_cha_spawn_allies` 池复活；不借用战术共享点，不因敌人推进改到对方基地。5秒冻结准备与背包窗口见[团队竞技流程](MATCH_FLOW_AND_SPAWNS.md)。

语音按本图NCS及实际 voice-assets 捕获准备，模式介绍绑定 `uk_1mc_charge`／`ab_1mc_charge`；4个场景物件按 props-assets、实体及注册槽位准备。完整声音与物件效果体验独立验收。

`native-acceptance.json` 的原生短局确认12名角色及动画树、非默认小地图、同一条命移动146.19单位、实际交战与12行结算。AAR地图ID9、模式ID15，比分1000:700；该局复用了上一局返回的冲锋房间，结算后原生箭头往返通过。`replay/1792021326049345000.json` 另采到58帧准备冻结位，冻结期间位置范围为0。完整倒计时观感、冻结换包及冲锋语音听感见 `REVIEW_PENDING.md`。

`normal-native-acceptance.json` 另确认正常配置、最终语音／物件准备下同生命移动101.41单位、弹匣30→28及换弹恢复30，原生快照含4个物件，实际音频通道观察到 `ab_spawn_music`。原生冻结房间分配的玩家对局阵营为1；第二条命首次观察位于 `[1392,1080,-58.95]`，与本队固定基地点水平位置相同，距开局阵型最近点251.85单位，采样时已超过15秒宽限。后续连续帧在移动之后取得，不能当成精确出生瞬间。

完整24份目录的后台加载和60秒模拟全部通过，加载204709ms；正常入口等待地图就绪的上限为240秒。架构、fmt、Clippy及167项Python检查通过。完整Rust检查仍有 `observe::enforces_byte_budget_and_idle_timeout` 失败，独立重跑也失败；其它Rust检查通过，未放宽该测试条件。正常账号资料哈希不变，9个自有诊断实验均结束并恢复临时CA，见 `owned-diagnostics-cleanup.json` 和 `normal-service-integrity.json`。

扩图使用 `prepare_tdm_map.py --mode cha` 和本模式实际初始化捕获，再执行 `prepare-map-starts`、语音／物件绑定、`verify-maps` 及独立原生短局。缺少冲锋 `_start` 时只使用同队真实冲锋基地点；没有冲锋点池则拒绝准备。采集参数为 `--resource-capture-map MAP --resource-capture-mode cha`，默认仍为 `war`。混合诊断目录可用 `verify_native_maps.py --mode cha` 选择冲锋，短局准备不会覆盖同图战术文件。
