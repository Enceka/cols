# 团队竞技开局与出生规则

本版原生自建房使用 `war`，中文资源为“战术团队竞技模式”，模式表别名为 `tdm`。冲锋团队竞技的独立 token 是 `cha`，中文资源为“冲锋团队竞技”。二者不能因 Rust 模块名 `tdm` 而共用一种出生规则。

`fixtures/protocol/team-modes-3.3.3.3.json` 保存本版只读模式表与中文资源证据。前端 `codo/constants/table/Mode.as` 的 NUMBER、ID、ALIAS、NAME 分别对应列 3、4、5、6；`war` 的 NUMBER 为 1，`cha` 为 15。原生 localize 类型 27 的名称读取器 `0x483d70` 使用 header+4，文本位于 header+0，中文 bytes 按 GB18030 解码。资源的 `loaded` 标志单独保留；缓存里可读到模式名称不代表该模式已实际进入或验收。`clash` 的缓存名称是“刀战”，不能套用其它版本的英文模式命名。

| 模式 | 开局 | 后续出生 | 当前接入 |
| --- | --- | --- | --- |
| 战术 `war`，别名 `tdm` | 本队 `mp_tdm_spawn_axis_start`／`mp_tdm_spawn_allies_start` | 开局宽限结束后从共享 `mp_tdm_spawn` 池按当前局势选点 | 原生入口和地图准备已接，流程／动态选点有后台验证；新流程视觉待复核 |
| 冲锋 `cha` | 本队冲锋开局点池；无专属开局组时使用同队真实冲锋基地点 | 宽限结束后从本队 `mp_cha_spawn_axis`／`mp_cha_spawn_allies` 池复活 | 当前版本具有双方原生基地点池的9图已完成资源、12人原生入口、短局结算及返回；无冲锋点池的地图排除 |

正常目录按 `(map, mode)` 接受独立准备的 `war`／`cha`，核弹小镇两种模式分别绑定实际初始化捕获和自己的点池。模式由外层原生 gametype 绑定，不能通过 `match_config` 内的字段覆盖。结算 NUMBER 来自规则的模式身份。冲锋资源、入口及实采范围见[冲锋团队竞技](CHARGE_MODE.md)。

## 开局与一局的生命周期

连接先完成认证／准入、档案和背包装配、gamestate 初始化、资源注册及首个世界快照确认。双方队伍和 Bot 的开局位置已经确定；纯加载期间不推进比赛。通过模型规则及首帧 ACK 门槛后，开始 5000ms 准备倒计时。

准备期间冻结移动、视角和武器操作，Bot 不移动或开火，战斗不扣血／计分。可靠背包菜单仍接受 `mr <serverId> 1 custom1..custom5`；武器注册可靠命令被客户端确认后，装备立即应用。枪械相同但技能、投掷物或奖励组合不同也立即应用，不需要制造一次死亡／出生。输入的 command time 正常确认，位移和武器时间不补跑准备间隔。

本版 `PmoveSingle 0x41dba0` 在 `0x41dbe3` 检查 PS.pm_flags 的 `0x800`，在 `0x41dcdd` 清除方向／动作输入和速度。服务端输入门槛与快照中的这一位同时生效，解冻后的过期输入仍按其执行时间拒绝动作。倒计时使用原生 HUD type2 的数字值（HUD+0x80／field9），`0x514b70` 以 `%g` 格式化；文字使用本图 NCS 的 `MP_MATCH_STARTING_IN`，不硬编码中文或跨地图文本槽。

倒计时结束才记录正式比赛起点；正常 600000ms 时限及 AAR 时长不包含加载和准备时间。玩家与 Bot 开始行动，同时进入 15000ms 开局宽限。宽限不是冻结：本条命尚未射击、近战、投掷或受伤的玩家，可以在资源 ACK 后即时换背包；参与战斗后或宽限结束后，背包留到下次出生。战术模式在这一宽限内仍使用本队开局点池。

宽限结束后，战术模式的死亡／复活使用共享动态点池。死亡、尸体、个人回放与跳过沿用各自状态；当前复活要求死亡至少 3000ms、个人回放不再 pending、对局未结束。出生应用已确认的背包，重置本条命战斗资格、状态效果、生命与视角。达到比分或正式时限后冻结计分／战斗，进入最终回放、前三名、结算和原生返回流程。新连接重新创建准备与比赛状态。

参考链路是 AlphaBuild 实际脚本树 `dedicated/bin/home/codol_rawfiles/maps/mp/gametypes/` 中的 `_gamelogic::waitForPlayers`／`matchStartTimerPC`／`prematchPeriod`／`startGame`／`gracePeriod`，`_menus::menuClass`，`_playerlogic::spawnPlayer` 和 `war::getSpawnPoint`。PC 默认最多等待双方出生 15 秒，双方已就绪则提前结束等待，再执行 5 秒准备；`level.gracePeriod` 为正式开战后的 15 秒。这些脚本提供流程依据，本版冻结位、HUD布局、菜单和模式身份独立核对，不能把 AlphaBuild 的运行结果当成 3.3.3.3 原生验收。

## 动态点池与安全判定

每份准备分别保存开局阵型 `spawns`／`spawn_angles` 和共享 `respawn_points`。实体按精确 map token 和 `script_specialopsname` 过滤；点位及朝向保持配对。准备工具以真实站立胶囊筛选开局／共享点，保存拒绝原因；运行中再次检查当前碰撞，跳过被动态物件阻挡的点，不编造坐标。

玩家和 Bot 使用同一选点入口。活人影响选点，死亡人物和复活者的旧身体不参与；连续 Bot 出生会考虑同 tick 已生成的身体。基础权重按 Alpha `_spawnlogic::getSpawnpoint_NearTeam` 的水平方向距离求和：`(敌人距离和 - 2 × 队友距离和) / 活人数量`。再加入敌人 1000 单位内的接近惩罚、实际射线暴露惩罚、人物占位和上一出生点惩罚。因此敌人绕后或推进会改变下一次出生的安全侧，不会把还活着的队伍直接传送到另一边。

旧准备缺共享池时，兼容路径仅把两队真实开局点合并作为有限候选；这不等价于完整原生共享池。正常地图的完整池应由 `prepare_tdm_map.py` 或 `prepare_match_flow.py` 绑定。导航范围同时覆盖开局与共享出生点，节点预算为 32768，避免扩大范围后截断另一端的队伍。射线使用本版世界／玻璃／物件碰撞；Alpha 的多次视锥采样、手雷／炮击危险、战术插入及其它特殊出生影响尚未完整恢复。

已有捕获的绑定命令：

```sh
python3 tools/prepare_match_flow.py \
  --catalog research-output/native-tdm-map-catalog.json \
  --capture-root research-output/gamehub \
  --output research-output/match-flow-new \
  --server target/release/codol-server
```

工具按原准备的实体 SHA 查找来源，按本图 CRC 核对已初始化 NCS，不读取账号；生成新文件及出生排除报告。`prepare-map-starts` 保持朝向索引，规范化资源引用，然后执行完整准备校验。

代码验证覆盖准备期限、正式时限、冻结输入／弹药／换枪、资源 ACK 后即刻配装、未交战宽限／受伤／射击关闭窗口、两阵营动态换点、死亡敌人排除、朝向配对及冲锋不换基地。原生只读模式资源与协议布局证据、后台地图模拟与实际新流程的视觉／听感验收分别记录；新倒计时、解冻、宽限换包和动态复活体验见 `REVIEW_PENDING.md`。

`evidence.json`及各图出生筛选报告记录23份准备、501个保留共享点（包含竞技场4人诊断准备，不代表PVE验收）。正常准备已绑定这些点和5000／15000ms配置，7500分／600000ms保持不变；原文备份和SHA核对见同目录`normal-integration.json`。原生状态采样的23份60秒后台模拟全部通过，合计245名Bot达到移动门槛、449次击杀和418次复活；这是后台功能证据。完整架构／fmt／Clippy／Rust／Python检查见`match-flow-final-check.log`，优化构建见`match-flow-release.log`。
