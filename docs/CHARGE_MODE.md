# 冲锋团队竞技原生接入

本版冲锋使用 `cha`、模式 NUMBER 15；战术使用 `war`、NUMBER 1，`tdm` 是战术别名。房间、资源目录及 Bot 外观按 `(map, mode)` 绑定，同图两种模式可以同时存在。地图名称使用本版已加载中文资源，见 `fixtures/protocol/charge-map-labels-3.3.3.3.json`。

默认 `research-output/native-tdm-map-catalog.json` 含23份战术准备及9份冲锋准备。原生入口为“玩 → 多人竞技 → 创建房间 → 冲锋团队竞技”，用地图预览箭头选已准备地图，再点准备，11名 Bot 加入组成12人房间。初选地图受原生保留状态影响，服务端不改本版开启标志。

核弹小镇准备为 `mp_nuked-ready-state.json`，正常7500分／600000ms。资源来自实际进入 `cha/mp_nuked` 的只读捕获：NCS初始化完成，实际CRC为1765105316；碰撞、武器、角色、服饰及真实小地图材质均核对本图身份。初始化探测元数据不参与正常游玩，也不作为资源准备证明。

本图提供9／9个冲锋开局点，站立碰撞筛选后的基地复活池保留8／9个点。开局和15秒宽限使用本队开局阵型，之后从本队 `mp_cha_spawn_axis`／`mp_cha_spawn_allies` 池复活；不借用战术共享点，不因敌人推进改到对方基地。5秒冻结准备与背包窗口见[团队竞技流程](MATCH_FLOW_AND_SPAWNS.md)。

语音按本图NCS及实际 voice-assets 捕获准备，模式介绍绑定 `uk_1mc_charge`／`ab_1mc_charge`；4个场景物件按 props-assets、实体及注册槽位准备。完整声音与物件效果体验独立验收。

`native-acceptance.json` 的原生短局确认12名角色及动画树、非默认小地图、同一条命移动146.19单位、实际交战与12行结算。AAR地图ID9、模式ID15，比分1000:700；该局复用了上一局返回的冲锋房间，结算后原生箭头往返通过。原生状态采样另确认58帧准备冻结位，冻结期间位置范围为0。完整倒计时观感、冻结换包及冲锋语音听感见 `REVIEW_PENDING.md`。

`normal-native-acceptance.json` 另确认正常配置、最终语音／物件准备下同生命移动101.41单位、弹匣30→28及换弹恢复30，原生快照含4个物件，实际音频通道观察到 `ab_spawn_music`。原生冻结房间分配的玩家对局阵营为1；第二条命首次观察位于 `[1392,1080,-58.95]`，与本队固定基地点水平位置相同，距开局阵型最近点251.85单位，采样时已超过15秒宽限。后续连续帧在移动之后取得，不能当成精确出生瞬间。

完整32份目录（23份战术、9份冲锋）有后台加载及逐图60秒模拟验证，9份冲锋亦单独检查。导航在条目首次创建对局时生成并缓存，见[启动边界](BACKEND_STARTUP.md)。原生短局验证12人模型、同命移动、模式15结算与返回；完整声音／画面与长期运行另做专项复核，诊断配置不覆盖正常账号和准备。

扩图使用 `prepare_tdm_map.py --mode cha` 和本模式实际初始化捕获，再执行 `prepare-map-starts`、语音／物件绑定、`verify-maps` 及独立原生短局。缺少冲锋 `_start` 时只使用同队真实冲锋基地点；没有冲锋点池则拒绝准备。采集参数为 `--resource-capture-map MAP --resource-capture-mode cha`，默认仍为 `war`。混合诊断目录可用 `verify_native_maps.py --mode cha` 选择冲锋，短局准备不会覆盖同图战术文件。

点池少于本队人数时，开局槽位按顺序复用本队真实点及配对朝向，基地池只保留碰撞有效的原始候选，不补造坐标。绿洲 `mp_mideast` 双方各有5个原生开局点和基地点，12人准备的每队第6个开局槽位复用首点；这与没有冲锋基地组的地图不同。只读碰撞／玻璃采集按本版地图表接受 `war` 或 `cha` 的PVP身份，冲锋专用图不再受战术模式身份限制，世界碰撞仍按实际BSP引用导出。

`audit_charge_maps.py` 重新读取实体源，核对客户端、精确地图及SHA，再区分已准备、有点池待准备、无点池排除和当前版本关闭身份；目录收录状态不替代原生玩法验收：

```sh
python3 tools/audit_charge_maps.py \
  --capture-index CAPTURE_INDEX_JSON \
  --catalog research-output/native-tdm-map-catalog.json \
  --output NEW_OUTPUT_JSON
```

正常冲锋地图及原生证据：

| 地图 | 原生ID | 实际CRC | 开局槽位／有效固定基地点数（Axis, Allies） | 原生短局证据 |
| --- | --- | --- | --- | --- |
| 核弹小镇 `mp_nuked` | 9 | 1765105316 | 9,9／8,9 | `native-acceptance.json`，模式15及房间复用 |
| 储油站 `mp_rust_long` | 11 | 1969495731 | 9,9／12,10 | `extension-native-pinned-acceptance.json` 的本图通过行；移动144.75单位、1000:900 |
| 雪山之巅 `mp_mountain` | 71 | 78003556 | 9,9／9,9 | `mp_mountain-native-final-1.json` 的对局采样及 `mp_mountain-native-return-acceptance.json` 的返回补验；移动190.24单位、600:700 |
| 高楼 `mp_highrise` | 50 | 4261926685 | 9,9／9,9 | `mp_highrise-native-final.json`；移动137.70单位、1000:600 |
| 废墟 `mp_ambush_sh` | 14 | 982376201 | 9,9／9,9 | `mp_ambush_sh-remaining-native-final.json`；移动152.45单位、500:700 |
| 混战码头 `mp_shipment_long` | 12 | 1725918291 | 9,9／10,10 | `mp_shipment_long-remaining-native-final.json`；移动158.69单位、900:1000 |
| 绿洲 `mp_mideast` | 51 | 1179086904 | 6,6／5,5 | `mp_mideast-remaining-native-final.json`；移动93.91单位、1000:800 |
| 修道院 `mp_cha_quad` | 52 | 1484676290 | 6,6／6,6 | `mp_cha_quad-remaining-native-final.json`；移动124.24单位、500:500 |
| 西部乐园 `mp_boomtown` | 54 | 4260250573 | 9,9／9,9 | `mp_boomtown-remaining-native-final.json`；移动110.01单位、700:900 |

各图均有本模式实际初始化NCS、实体、世界碰撞、武器、角色及服饰捕获，12名角色／动画树、非默认小地图、同生命移动、本局结束帧、12行结算、模式15和返回房间通过。高楼没有单独的冲锋 `_start` 组，开局使用本队真实冲锋基地池；仍按本图朝向和碰撞筛选。两份基地池不同于战术共享池。正常配置7500分／10分钟，原战术条目及赛单保留。

正常集成及短局与正常资源等价核对见 `remaining-normal-integration.json`，后台报告为 `remaining-mixed-server-acceptance.json`。废墟、混战码头、绿洲、修道院、西部乐园在同一原生客户端及房间内连续通过，普通帧和本局结束帧分别保留；12行AAR的地图／模式身份及双方比分均与服务端对应局一致，见 `remaining-native-acceptance.json`。这五图使用固定的已核对玩法构建，指纹见 `remaining-source-provenance.json`；当前工作区的奖励及共享真人对局改动只纳入完整目录后台检查，不属于这五局原生玩法的通过范围。储油站、雪山之巅、高楼的独立构建及返回补验见 `extension-normal-integration.json` 和 `pinned-source-provenance.json`。旧结算被拒绝的失败样本仍保留于 `extension-native-acceptance-v4.json`，雪山之巅首次返回失败与返回补验分别留存。

本版当前版本列开启11个冲锋地图身份，其中9图具有双方原生基地点池，均已完成资源及原生短局接入。航站楼、潜艇基地没有冲锋基地组，排除冲锋接入且不列为待补任务，现有战术准备保留；另外6个当前版本关闭身份维持原生标志。逐图覆盖及原始点池计数见 `fixtures/protocol/charge-spawn-coverage-3.3.3.3.json`，实体来源与SHA见 `extension-spawn-resource-audit.json`。没有点池的地图不接入，不借用战术点池或编造坐标。

默认原生入口引用的本地研究证书已续期；过期原件保留在 `expired-default-tls/`，校验与安装摘要见 `default-tls-renewal.json`。证书和私钥仅留本机，账号及客户端连接配置仍由会话启动／结束时临时应用并恢复。
