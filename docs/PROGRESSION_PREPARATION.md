# 人物与武器经验、结算与提示

目标为未修改的 CODOL 3.3.3.3 客户端、纯 Rust 服务端。已接入单文件经验事务、完成TDM对局后的权威击杀奖励、人物／武器原生更新命令、人物升级splash及可靠投递确认。新业务默认无发奖策略，必须显式提供本地 `progression-policy.json` 才启用；当前只在隔离诊断目录启用。人物原生重登、M9等级2显示和M4／M9库存XP读取已验证；阿富汗真实碰撞／武器参数的完整服务端击杀→升级→重启／重试链路已验证。战斗内升级动画和武器结算条仍待实机验收。

新结构的职责与依赖见[模块结构](SERVER_MODULARIZATION_PLAN.md)。账号事务位于 `backend/progression/service.rs`，共享奖励输入位于 `contracts/progression.rs`；`app/services`只组合接口，`transport/lobby/handlers`只编码响应。没有把账号文件或业务服务放进玩法世界。

## 已确认的本版规则

| 项目 | 人物等级 | 主武器等级 | 副武器等级 |
| --- | --- | --- | --- |
| 显示等级 | 1～70 | 1～31 | 1～20 |
| 原生索引 | 0～69 | 0～30 | 0～19 |
| 最高等级起始累计 XP | 3582550 | 121230 | 71200 |
| 保存的经验单位 | 当前等级内经验 | 库存中的累计经验 | 库存中的累计经验 |
| 数据来源 | private 档案 | marketplace 物品记录 | marketplace 物品记录 |

人物等级使用 `mp/rankTable.csv` v4，不能移植 Alpha 的 v2 元数据。`rank` 为显示等级，`experience` 为级内经验；累计 XP 为本级门槛加 `experience`。例如累计 2001 XP 必须编码为 rank=2、experience=1。`rankVersion=4` 是表版本，转生是另一个 `prestige` 字段。

武器使用 `mp/coWeaponPrimaryRankTable.csv`／`mp/coWeaponSecondaryRankTable.csv` 两套曲线。主武器最后门槛 121230 不同于 Alpha 的 179600。`WL_GetWeaponRank` 的本版链为 VA `0x6b7190 → 0x695bc0 → 0x695a80 → 0x46eb20`，最终显示值加 1；库存记录的 XP 为累计值，不应写入 private 的武器统计数组。

经验必须按 marketplace item ID 保存。已整理本版 201 个 `wea` 物品，例如 M4=`10001`、M9=`10049`、AK47=`10009`。注册武器索引、统计表编号和附带配件的组合武器编号都不是该 ID。本版 item record +6 的分类字节及 `0x6b7190`／`0x6b5120` 的 `<6` 比较已互证，201个武器分类均已生成；皮肤／变体是否共享经验仍未恢复，当前按实际物品ID独立保存。

## 已准备的代码与文件

- `crates/server/src/backend/progression/mod.rs`：校验本版三套曲线，累计／级内经验互转，纯内存计算经验奖励，输出升级前后状态及每个跨越等级。满级没有“下一级”，有符号XP溢出受限。
- `crates/server/src/game/progression/mod.rs`：玩法层收集服务端已确认的敌方击杀，以受害者死亡序号去重。人物／武器经验倍率来自显式策略，基础物品ID来自实际配装；自杀、友方击杀、坠落及Bot自身击杀不发玩家奖励，未映射、未拥有、近战／投掷物／奖励设备不能借用当前步枪ID发武器XP。
- `crates/server/src/transport/game/connection/progression.rs`及`transport/game/server/mod.rs`：完成对局后提交一个稳定批次，失败时保持相同match ID重试，并延迟原生结算。提交成功后发送K私有档案、武器累计XP及人物升级提示，再由实际可靠命令ACK确认回执。INIT重发复用连接，不建立第二笔经验；此处描述单人Bot对局的经验接入；共享世界已独立实现，但共享多人经验尚未完成等同验收。
- `crates/server/src/backend/progression/service.rs`：在共享账号写锁下提交人物进度、武器XP和原始回执，同一match ID与相同奖励重试返回原回执，奖励内容冲突则拒绝。提醒候选随回执保存，实际投递确认后才能持久化ack；入队不视为已送达。窗口写满时把已确认前缀释放到归档段，再继续提交。
- `crates/protocol/src/lobby/profile.rs`：`player_progress` 读取校验归属后的进度；`with_player_progress` 准备 v4 更新，重算 CRC 与 36 项投影，保留转生和其它字节，并提高 `uf_prestige_rank` 的普通等级下限而保留已有更高值。正式调用前必须先用等级模型校验／计算，不能直接把累计 XP 当参数传入。
- `crates/protocol/src/lobby/task.rs`：`InventoryItem { id, xp }`、`player_data_with_inventory` 和 `inventory_records_with_progress`，分别覆盖初始 `80/230` 内嵌库存及 `80/203` 独立刷新记录。两种响应的归属编码不同，现有业务均已消费同一已提交经验投影；没有经验文件时保持原零XP行为。
- `crates/protocol/src/game/snapshot/progression.rs`：本版二进制K补丁只写rank／级内XP／表版本和rank unlock值；`< <itemID> xp <累计XP>`更新武器查询缓存，`$ 0 0 <rankIndex>`使用promotion行0，`ui_promotion=1`通知原生结算。写入绝对值，可靠重发不会再次增加XP。
- `tools/prepare_progression.py`：从只读本版 capture 生成 `curves.json` 与 `catalog.json`，核对表版本、顺序、级间跨度、schema 偏移、rank flag 和物品 ID，记录来源哈希及缺失证据。输出目录要求不存在，输入 capture 保持不变。
- `tools/ClientAssets.cs`：只读导出武器分类、兼容件及商品关联，并采集 `mp/rankIconTable.csv`、`mp/splashTable.csv`。这两张表在大厅可保留资源对象但未设置loaded旗标，因此只放宽这两项的loaded检查，保留身份、尺寸和读取限制；探针同时导出loaded状态，不把对象存在当成已运行原生提示。`--profiles`增加固定M4/M9的XP查询及查询表状态，不读取会话密钥。
- `diagnostics/progression.rs`及 `commit-test-progression`：只在有明确诊断标记的独立服务目录离线提交合成结算，拒绝缺少标记的目录；不接客户端发奖RPC。现有 `boost-test-account`拒绝覆盖已有经验事务的导入种子，需先设计显式迁移。
- `fixtures/protocol/progression-3.3.3.3.json`：只有本版数字门槛及样本身份，用于全部等级边界的回归；没有账号数据、完整游戏资源或本地路径。

private v19 的偏移均包含 CRC：experience=4844、prestige=4883、rank=4887、rankVersion=4891、`unlockFlagsNew[1]`=8297（Int16，`uf_prestige_rank`）。准备器从 runtime schema 独立核对，而不是沿用旧版地址。

在 `server/` 下重现：

```sh
python3 tools/prepare_progression.py \
  --assets ASSETS_JSON \
  --output research-output/progression-preparation-new
sh tools/check.sh
```

本机准备结果包含201个原生武器分类和2716条配件等级观察（170个武器）；`progression-rules.json`只启用曲线选择，配件提醒仍为空。原始采集、诊断账号及验证结果位于本机忽略目录。

```sh
python3 tools/prepare_progression.py \
  --assets ASSETS_JSON \
  --with-runtime-rules --output research-output/progression-rules-new

# 只在独立诊断服务目录有 isolated-offline-progression-v1 标记时执行。
sh tools/cargo.sh run --locked -p codol-server -- commit-test-progression \
  --service-data-dir NEW_OUTPUT_JSON \
  --input NEW_OUTPUT_JSON
```

## 经验事务与投影

每个title/UID的 `profiles/<title>/<uid>/progression.json`是该账号人物经验、武器累计XP、结算回执与提醒确认状态的唯一提交记录。原 `private.bin`作为校验过的导入种子保持不变，原库存和钱包也不改写。文件包含完整带CRC／投影的私有档案、每个武器的XP和修订号；同一rename提交所有字段，Unix上同步父目录。跨进程重启后，重复结算仍返回相同回执。

所有读取检查账号／样本、种子哈希、CRC／投影和修订历史；逐条重算回执并核对最终XP与档案。外部改变种子或升级规则需要显式迁移，当前拒绝静默重解释。私有档案RPC目前只读，游戏stats上传也不直接写人物权威进度；公有档案装配保存保持现有接口。

`ProfileService`的私有读取、`LocalServices::game_profile`和库存两种响应都使用该提交记录。发武器XP前同时核对基础库存和永久购买所有权；经验不会授予武器／配件。一个批次中的未知、未拥有或重复武器导致整笔提交失败，不留下部分人物经验。每笔回执保留所有跨越等级，只有尚未确认的回执才进入pending结果。

当前回执上限1024、文件上限4MiB；到达上限时先把已确认的连续前缀整体写入不可变归档段`progression-archive-NNNN.json`，再继续接收新提交；未确认的回执留在窗口内，旧重试记录不会因归档消失。归档段自带种子／规则摘要、起止修订号与前后累计XP，读取时校验段摘要、修订链和累计值，归档回执的重试从段内还原原回执；账本另存最近4096条match ID与奖励摘要的墓碑作为快路径，墓碑被裁剪后仍会回读归档段判定重试，不会重复发奖。也可用`codol-server archive-progression --service-data-dir <root> --title-id <title> --user-id <uid>`显式归档一次。当前API接收的是一局最终确认的奖励，不应被调用成每帧或每次ACK发奖；对局ID仍需要由玩法／应用边界稳定生成。

## 完成对局的发奖策略

`contracts::progression::Policy`是共享的明确本地规则，不从记分板分数或客户端输入推断。`backend/progression`核对文件版本／样本／数值和武器分类规则，接入边界只为实际TDM世界启用；训练和资源采集不启用。

隔离诊断使用：

```json
{
  "schema_version": 1,
  "client_sha256": "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
  "enabled": true,
  "awards": {
    "kill_player_xp": 100,
    "kill_weapon_xp": 100,
    "headshot_player_bonus": 0
  }
}
```

100／100是本地TDM策略，不声称已恢复官方倍率。只提交完成的对局；未完成退出不结算。每个完整奖励批次有随机128位ID，提交失败保留同一ID；目前未承诺断线后恢复同局奖励，不能把新准入当作原对局恢复。武器皮肤／变体仍按实际marketplace ID独立计入，主副／下挂共用基础武器ID；配装切换后用当前生效配装解析击杀武器，不读尚未出生生效的背包。

场内记分板XP计入本局已确认奖励；等级／武器累计XP在完成并持久化后更新。一次跨多级回执保留全部新等级，场内人物提示合并为该批次的最终级别。未确认的提醒回执保留在pending列表；当前未增加跨会话补播UI，不能将其当成已显示。普通账号没有显式策略时不自动发放经验。

### 可靠窗口满时的结算重试

最终比分、参赛者信息和胜负／地图／模式字段作为一批准备，检查可靠窗口剩余空间后，在队列副本上完整入队再发布。空间不足时不改变比分脏标志、发送标志或结算序号，快照轮询仍继续发送已有可靠命令，收到ACK后再重试；不能因队列满直接退出轮询，也不能先把比分标为已发送而遗漏后续结算字段。TDM等待期间不会进入单目标训练的比分分支。

最终结算是否待发送独立于比分脏标志检查。升级K／武器XP／人物提示使用整批入队语义，最后一条命令未ACK时不确认回执；最终比分批次未全部ACK时不打开原生结算页。新增回归用接近满的真实可靠队列验证失败无部分发布、快照仍发送、释放空间后只发布一次，以及部分ACK不会提前确认或打开界面。

胜负音乐／语音和前三名卡片／打开菜单分别整批入队；空间不足时保留待发送状态。关闭前三名菜单、打开结算页和返房命令同样等待窗口空间，避免比分批次刚好填满窗口后，由后续表现命令再次中断快照轮询。

## 升级与配件提醒的原生入口

结算前端已有两类消费者：

- `SummaryBaseController.as` 通过 `FetchGameSummary` 展示人物经验，通过 `WL_GetWeaponCareerInfoAtLevel` 查询某武器下一等级的可用内容。
- `LobbySummaryController.as::get_weapons` 调用 `AS_GetWeaponLevelup`，消费 `progressInfo.currentLevel / targetLevel / srcExpValue / desExpValue`、`isPrimary` 和 `unlockAttachment`，分别加载主／副曲线。因此本版确实预留了武器升级及配件解锁展示，不需要先增加自制界面；数据产生、刷新和消费时机仍待验证。

`AS_GetWeaponLevelup` 的本版绑定为 `0x5b6ab0 → vtable 0xc6dd48 → 0x5a9cc0`。后者先刷新新缓存，再按 120 个 12 字节记录比较旧缓存 `0x2f24760` 与新缓存 `0x2f24d00`：+0 为物品ID、+4 为累计XP、+8 为等级索引；要求同一ID且新XP更高，输出等级加1，并去除重复武器。故仅更改大厅库存不足以证明结算会正确展示：还要恢复旧缓存的形成时机、刷新顺序和解锁列表的实际生成路径。当前判断来自静态消费链，缓存生产时序尚待原生采样。

本版静态消费链确认`AS_GetXPGPInfo`的vtable `0xc6dd54`调用`0x5aa290`：`0x590810`在19行`aarScoreboard`中查找`isLocalPlayer`行，`xpGained`读取该行的`xp`；`currentRank`由`0x590720`读取private的rank并减1，`totalExp`由`0x590550`结合private级内经验和等级表计算。这与现有J记分板XP及K私有进度的职责一致，人物结算无需另造经验摘要命令。武器旧缓存的生产调用已定位到原生回调`0x691730`的非零分支：调用`0x6bf930(0x2f24760)`；该生产器遍历60组主副武器，保存120条记录。回调在实际入场／返房中的调用时序仍待实机采样，不能把静态写入点当成缓存验收。

战斗内splash入口：本版server-command分发VA `0x57d560`的`$`分支调用`0x57c860`，读取`mp/splashTable.csv`并设置五个槽之一。新采集表为518×12，人物`promotion`为行0，持续4秒、声音`mp_level_up`、专用`promotion`绘制。已接入`$ 0 0 <rankIndex>`和原生结算的`ui_promotion=1`；参数、可靠编码及ACK经过自动验证，实际图标／声音／动画仍待验收。当前版本没有Alpha的`promotion_weapon`行，`asym_weapon_level_up`属于其它模式，不套用作普通多人武器升级。

私有档案更新走原生二进制可靠K：`0x57e330 → 0x57bf70(1)`，布局与公有J相同，由`offset LE32 / length u8 / bytes`组成，偏移相对于不含CRC的数据区。经验4840、rank4883、版本4887、rank unlock8293；原生消费者从`0x6da140()+0x91b8`写入。武器更新的`< item xp total`由`0x57cac0`写缓存+0x18。它们使用已保存的值，不依赖客户端计算或客户端上传。

武器查询缓存在初始库存之后建立，key按本版 `ror8(itemID) XOR mask` 转换。探针只在本地计算key、不输出掩码；M4／M9初始和查询缓存各500XP、M9原生等级2已有读取和仓库画面证据。

本版配件链已经定位：`WL_GetWeaponCareerInfoAtLevel`绑定 `0x6a6690 → vtable 0xc7aa28 → 0x6b5120`，显示等级减1后进入`0x6bf700`；武器详情 +0x44/+0x48为数量／12字节兼容记录。`0x706c90`读取配件关联Product列表，将Product +4的UInt16与武器等级索引比较。已采集2716条非零门槛观察，M9扩容弹匣`21163`的Product2611保存19，对应显示等级20，与已有界面锁20证据一致。

同一配件可能还关联level0的其它购买渠道，原生列表同时通过`0x706d90 → 0x707040 → 0x7074e0`检查SKU可用性／价格／时间。因此这些观察尚不能整体转换为“第一次可用”的提醒规则。插槽门槛也仍待恢复；配件门槛、可选渠道、插槽解锁和获得所有权需要分别处理。旧Alpha配件表不作为本版规则来源。

提醒应使用同一已提交奖励记录生成：人物升级、武器升级和配件候选包含明确的旧／新状态，一次跨多级列出所有新等级。已有可用配件不会因同一物品重复门槛再次成为候选。正式提示的去重键需要包含账号、对局／事件身份和奖励身份，并在重连后明确重放／确认规则；内存中的候选计算不等于持久化去重。

## 验收要求与剩余范围

1. 使用独立低等级诊断账号和独立服务目录采集实际 weapon primary/secondary 分类、配件条件、插槽门槛及 splash 表。普通账号已有等级 70、全物品库存，无法直接验证自然人物升级；保留它的进度。
2. 以服务端确认的击杀／目标事件确定人物与武器经验，携带基础物品 ID。核对持枪击杀、爆炸物、连杀奖励、拾枪、换背包、友伤、自杀和 Bot／训练模式各自是否计入；不把记分板分数直接当作通用 XP。
3. 已接入完成的本地TDM对局奖励及稳定提交／失败重试。捕获地图验证实际M4发射／命中／击杀产生100人物XP和100武器XP，重启、重复请求和ACK均验证。已确认回执的归档与段内重试还原已覆盖；未完成退出、进程终止前尚未持久化的奖励和跨连接世界恢复目前没有发奖保证。
4. 人物／武器档案、两种库存响应、准入名片读取、K／武器XP／人物splash和AAR标志已接。原生M9等级2和武器XP缓存已确认；继续验收完整对局后的场内动画／声音、武器结算进度条和返回大厅刷新。普通多人武器splash与配件提醒仍需确认本版实际入口。
5. 原生验收人物 1→2、武器 1→2、一次跨多级、满级、配件／插槽出现、结算返回、重登和对局重试；同时核对原昵称、装配、所有权及转生保留。还需证明没有重复发经验或重复提示。

## 验证与证据

自动验证覆盖全部经验曲线边界、累计／级内互转、跨多级、满级／溢出、CRC／投影／归属、无关字节保留、两类库存XP、所有权、并发／冲突重试、持久化提醒ACK和回执上限。可靠窗口压力验证升级、最终比分、胜负音效与前三名批次：不足时无部分发布，快照继续推进，ACK释放后只发一次。

真实捕获阿富汗的完整检查：

```sh
sh tools/cargo.sh test --locked -p codol-server captured_afghan_shooting \
  -- --ignored --nocapture
```

该检查使用实际碰撞／M4参数和隔离账号，4发击杀后以100分完成对局，人物1900→2000XP、M4 490→590XP均升到2级。重启、相同奖励重试、RPC／准入投影和可靠确认后，修订与经验不重复增加。证据文件 `captured-authority-proof.json` 明确 `native_client_used=false`；这是本机忽略目录中的真实资源服务端证明。

原生人物重登读取rank2／级内1／版本4，以及M4／M9各500XP和M9等级2已有证据。尚未验收完整对局后的场内升级动画／声音、武器结算条与配件提醒；静态消费者和原生库存显示不能替代这些结果。启动环境诊断见[客户端恢复](CLIENT_STARTUP_RECOVERY.md)。
