# 死亡武器掉落与拾取

目标为未修改的 CODOL 3.3.3.3 客户端和纯 Rust 服务端。当前已核对本版地面武器、组合模型、拾取资格、预测补弹和原生拾取／换枪提示的静态消费者，并对照 AlphaBuild 的死亡脚本与原生物品实现。Rust 已接入普通枪械死亡掉落、原生拾取提示、F换枪、兼容弹药拾取及回放归档；自动回归和真实采集资源集成测试通过。本版阿富汗原生对局已通过地面组合模型、拾取提示、F换枪和部分弹药回取；声音专项与更多地图仍待验收。

## 结论与证据边界

普通枪械可以沿本版 `ET_ITEM=3` 表达，保留完整组合武器编号，由客户端恢复枪体和配件。现有拾荒包证明 ITEM 传输可用，但其静止位置、自动补给、固定模型和寿命不能直接覆盖普通掉枪。死亡时封存武器／弹药、抛落碰撞、使用目标选择、补弹与换枪分流、库存和回放状态均由 Rust 持有。

本版地址均为 VA，ImageBase 为 `0x400000`；样本 SHA-256 为
`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。
字段、指令锚点、参考源码指纹及未验证项见
[`weapon-drops-3.3.3.3.json`](../fixtures/protocol/weapon-drops-3.3.3.3.json)。
静态消费者能确定客户端接受什么，不能单独证明官方服务端何时掉枪、保留多久或怎样扫描使用目标。

## AlphaBuild 的完整链路

参考根目录为 `../CODOL-AlphaBuild/CODOL_AlphaBuild_LAN 服务器端/CODOL_Server/dedicated/`，对应旧版 1.1.1.13。有效脚本参考在 `bin/home/codol_rawfiles/`。

```text
_damage.gsc::PlayerKilled_internal
  → dropScavengerForDeath(attacker)
  → dropWeaponForDeath(attacker)
    → player.dropItem(weapon)
      → CODOL_PlayerCmd_dropItem → Drop_Weapon → Drop_Item
    → item.ItemWeaponSetAmmo(right_clip, stock, left_clip)
    → watchPickup / deletePickupAfterAWhile

Drop_Item → ET_ITEM + 初速／重力 + owner短暂排除 + 掉落物队列
G_RunItem → 轨迹求值／碰撞／落地／越界或卡住清除
Touch_Item_Auto → 同枪或兼容枪补弹
使用操作 → Touch_Item → 掉出旧枪、获得新枪、移交弹药、选择新枪
```

`_damage.gsc:683..684` 要求先掉拾荒包，再掉武器，因为后者会从玩家持有集合移除武器。`_weapons.gsc:511..550` 在出生和 `weapon_change` 时维护 `lastDroppableWeapon`。下挂榴弹／特定配件模式先归一到当前主形态；临时手持奖励设备不应覆盖最后一把普通枪。

| 参考行为 | 源码依据与含义 |
| --- | --- |
| 每条生命只掉一次、一把 | `dropWeaponForDeath:654..734` 使用 `droppedDeathWeapon`，不遍历整个背包 |
| 选择最后一把可掉武器 | `mayDropWeapon:639` 排除 `none`、名称含 `ac130` 以及非 `primary` inventory type；这里是引擎物品类型，不能直接等同背包的主武器槽 |
| 开局宽限不掉枪 | 检查 `level.inGracePeriod`，不是写死“死亡时间小于某毫秒数” |
| 必须仍然持有 | 检查 `hasWeapon`；普通枪要求 `AnyAmmoForWeaponModes` 且左右弹匣至少一侧非零；只有备弹而双侧空匣也不掉 |
| 弹药保存 | 在 `dropItem` 移除持有权前读取右匣、左匣和备弹；备弹按 `WeaponMaxAmmo` 钳位，再调用 `ItemWeaponSetAmmo`，不自动补满 |
| 盾牌特例 | `riotshield_mp` 绕过普通弹药资格并写 `1,1,0`；独立刀具、特殊模式不能据此推定完成 |
| 寿命 | `deletePickupAfterAWhile:796` 等待 60 秒后删除；这是参考脚本值，本版官方寿命未恢复 |
| 所有权 | 设置原持有人和击杀者；`watchPickup:816` 区分仅取弹药与真正换枪，并在交换出的武器上延续原持有人信息；另有 GPS 持枪者追踪 |

普通掉枪函数没有拾荒包的“必须有攻击者且不能自杀”检查。把当前拾荒包的资格条件原样复用，会把普通掉枪规则混在一起；本版自杀、环境死亡、核弹等最终范围仍需分别核对。

`src/codol/weapon/codol_weapon_give.c:130` 默认从 `tag_weapon_right` 放出，`Drop_Weapon` 尝试使用骨骼世界矩阵，并从身体中心到标签位置做碰撞限制。标签失败有回退位置，不能让掉落完全依赖骨骼可用。

`src/asmsource/_g_items.asm:2188` 生成真正的 ITEM；`g_sv_main.c:719..729` 的默认掉落数上限为16、允许范围2..32，前向速度10、竖直基值10／随机5、水平随机100。满队列的回收扫描考虑掉落物到活跃玩家的最近距离，并选更远的候选，不是简单按生成时间删除最老一把。`DroppedItemClearOwner:1942` 与 handler16 在约1000ms后解除原掉落者排除；GSC记录的原持有人信息与这个物理拾取排除字段是两回事。

这些初速、数量、回收方式、标签与1000ms均为 AlphaBuild 的实现依据，尚未证明与本版官方生产逻辑完全一致。`G_RunItem` 具有重力、胶囊扫掠、落地／短时停止和无效位置清除；其保留引擎的轨迹枚举与布局不能直接搬入本版快照。

参考实现也有需要审查的边界：普通枪分支在 `dropItem` 后直接调用 `ItemWeaponSetAmmo`，没有盾牌分支的未定义结果检查；`Drop_Weapon` 对 clip-only、替代形态和共享弹药还另有资格与转移逻辑。移植时必须处理生成失败和双手／替代弹药，不能把脚本片段当作完备事务。

## 本版 ITEM 与组合枪模型

| 本版消费者／字段 | 已确认内容 |
| --- | --- |
| ITEM 字段表 `0xc543b8` | 61项，`ES+0x98` 为第19项、11位 packed selector；`ES+0x80` 为第30项、11位 owner |
| ITEM 角度字段16..18 | `ES+0x48 / 0x4c / 0x50` 使用 `-78`；`0x5a27f0` 分派到 `0x5a313d`，直接读取绝对 LE32 浮点角度，无控制位、无基线异或 |
| `0x501550` | 对 ITEM 的 `ES+0x98` 除900：余数为武器编号，商为模型槽，随后进入武器模型构建 |
| `0x501320` | 普通定义从 `WeaponDef+0x18+4×slot` 取世界模型；`*` 开头的组合定义进入专门构建分支并归零模型槽 |
| `0x42e6d0` | 从组合记录展开模型、标签及附属数据，供世界武器 DObj 构建；不能只发基础武器编号代替完整组合编号 |
| `0x412600` | 预测接触范围为玩家位置减 ITEM 位置：X/Y `[-36,36]`、Z `[-88,18]`；这不是已恢复的按键使用扫描半径 |
| `0x4128c0` | 拾取资格检查，包括 `PS+0x348` 的 `0x80` 位、owner、自身武器及备用模式；拾荒包为独立分支 |
| `0x4127d0 / 0x412740` | 武器／offhand类型资格，与持有状态、原生选择槽和武器类型有关 |
| `0x42f420 / 0x4126a0` | 兼容弹药按真实 ammo key 判断；补充余量并非“同一个商城物品ID”即可推定 |
| `0x4514f0` | 客户端接触预测：普通武器还要求已经持有、可补弹及其它资格；发出原生事件16，不能据此认定新枪所有权已移交 |

AlphaBuild 的 packed selector 使用 `weapon + 700×slot`，本版使用 **`weapon + 900×slot`**。本版该字段仍只有11位，必须检查整体编码不超过2047，不能假设任意16个世界模型槽均可发布。普通组合定义走slot0，可保留完整组合编号；无需用 GENERAL 实体的模型NCS编号替换它。

事件表确认 `EV_ITEM_PICKUP=16`、`EV_AMMO_PICKUP=17`。两者名称和预测补弹调用均有本版证据；换枪后具体事件组合、声音和回声去重仍需实采，不能直接沿用 Alpha 保留汇编里的事件数字。

## 本版原生拾取／交换提示

`0x44b470` 将预测PS的提示字段复制到客户端缓存，`0x44bc10` 绘制提示，普通武器进入 `0x44b520`。后者使用 `+activate`，按持有普通枪数量与原生上限选择 `PLATFORM_PICKUPNEWWEAPON` 或 `PLATFORM_SWAPWEAPONS`，并查询武器图标／名称。

| 语义 | 本版PS偏移 | 本版字段序号／编码 | Alpha旧版PS偏移 |
| --- | --- | --- | --- |
| cursorHint | `0x204` | 67／11位 | `0x208` |
| cursorHintString | `0x208` | 80／signed8 | `0x20c` |
| cursorHintEntIndex | `0x20c` | 78／11位 | `0x210` |
| 另一种图标绘制分支 | `0x210` | 98／1位、hint2 | `0x214` |

普通武器提示使用 `cursorHint = weapon + 4`；绘制范围5..904（`0x388`），再减4查武器定义。应使用武器编号，不能把 ITEM 的 packed selector 或商城物品ID直接加4。自定义字符串的无值为-1；无目标实体使用2047。第四字段已定位消费者，但原生服务端生产条件尚未恢复，不能给它擅自定义“是否长按”的含义。

已有本版输入实采表明默认F对应 `+activate`／`buttons & 0x8`，见 `crates/protocol/src/game/channel/usercmd.rs`。这能复用输入解码；具体扫描距离、视线、目标优先级和按下／持续时长仍需独立恢复。单纯发布提示字段不会让客户端替 Rust 修改库存。

## Rust 接入与行为范围

| 位置 | 当前实现 |
| --- | --- |
| `game/weapons/held/dropped.rs` | 每条生命只封存一次死亡枪；空匣拒绝；临时奖励设备取其保存的普通枪；变形归一到父形态并携带替代模式、真实弹药、射速／伤害／换弹／移动参数；拾取保留拾取者技能与投掷物 |
| `game/world/weapon_drops.rs` | 独立世界集合，16把／60秒、按离活跃玩家更远的物品回收；重力、碰撞、落地、支撑消失后继续下落；原掉落者1000ms排除；同ammo key自动取弹与F换枪分流 |
| `game/modes/tdm/combat.rs`、`transport/game/connection/weapon_drops.rs` | 读取死者当前普通枪和弹药，使用实际开局宽限；玩家／Bot死亡接入；重复击杀排空不重复生成；不会把凶器当掉落枪 |
| `crates/protocol/src/game/snapshot/weapon_drops.rs` | ITEM3实体1024..1087；组合编号保留slot0、位置／原始浮点角度、owner、编号代际和显式删除；命中事件分配器避开此区间 |
| `crates/protocol/src/game/snapshot/mod.rs` | 发布本版提示字段67／78／80，字符串-1；第四个hint2图标字段保持0；提示值为实际组合编号+4，消失时清除 |
| `transport/game/connection/weapon_drops.rs` | F新按下沿完成交换，旧枪留在地面；可靠命令`a <weapon>`同步客户端选枪；可靠窗口满时延后整个拾取，避免只有服务端换枪 |
| `crates/protocol/src/game/snapshot/corpse.rs`与`crates/protocol/src/game/snapshot/held_weapon.rs` | 保留尸体动画使用的武器定义，但已掉枪的尸体不再生成重复手持枪模型；前三名姿态仍正常持枪 |
| `game/presentation/replay.rs`、连接快照历史 | 归档对应帧的地面枪集合，使用已确认基线编码；个人／最终回放读取历史，期间禁止实际拾取并隐藏使用提示 |

换枪保存右弹匣和备弹，不补满；同ammo key仍有其它持有枪时，旧枪仅带走剩余持有容量之外的备弹，避免复制同一个共享池。拾得武器属于当前生命，不修改账号背包或出生装配模板；普通重生恢复保存的背包。

扫描使用明确的服务端策略：距脚下96单位、位于前方并有地图视线，按F新边沿交换。抛落从脚下+32单位开始，使用固定前向10／竖直10初速、800重力与半径1扫掠；地面姿态使用侧放角度。模型动画由本版原生消费者处理。该有界物理与扫描策略尚不等同于已经恢复的本版官方生产算法；Alpha的随机初速、骨骼标签位置、复杂移动支撑物与精确摆放仍待原生对照。

当前完整弹药移交范围是现有普通单弹匣枪，以及已经支持的共享弹药变形模式。双持独立左右弹匣、独立弹药替代模式、盾牌／发射器特殊资格和GPS追踪仍未扩展；不将普通枪接入当成这些武器的验收。Bot会生成自己的死亡枪，主动扫描／拾取仍由玩家输入驱动。

## 验证

协议回归按本版字段表独立解码，覆盖所有bit-pool起点、创建、位置／角度更新、交换、owner清除、编号复用与删除，以及提示字段和清除。ITEM角度使用绝对LE32浮点数；实体1024..1087置于较低编号命中事件之后、较高编号命中事件及声音／拾荒包之前。混合删除回归确认原生基线合并不会先继承地面枪再遇到其更新或删除，回归验证编号1024..1087与更高事件编号的合并顺序。服务端覆盖空匣、部分弹药、变形父枪、临时奖励设备、共享备弹、不超过容量补弹、死亡拒绝、原掉落者期限、F边沿、碰撞落地、到期和数量限制；尸体已掉枪后不再重复持枪。

`captured_shooting_death_drop_and_use_exchange_preserve_real_weapon_identity` 使用本机采集的阿富汗碰撞、武器装配和只读账号副本，执行实际武器射击／判伤／死亡队列／掉枪／提示／F换枪／可靠选枪与快照生成。证明掉落的是死者组合编号361，凶器为352；在死亡时刻2300生成实体1024，拾取弹匣25／备弹75，旧枪352保留射击后的弹匣17并回到地面。测试还覆盖重复排空、F重复输入，以及可靠窗口满时拒绝半次交换。证据在 `captured-gameplay-proof.json`。这些是实际采集资源上的服务端集成验证，不能替代原生画面和声音。

原生验收证据在 `native-proof.json`，对应隔离实验 `experiment.json`。未修改的本版客户端通过普通射击产生死亡掉枪，实体1025保留死者的组合编号358（AK117），凶器为352（AK117熔岩）；客户端显示地面模型、武器图标／名称和“按F更换武器”提示，cursorHint为362。F拾取后实际切到358，弹匣30／备弹90，旧熔岩枪留在地面；原持有人短暂排除结束后，提示变为356，再按F切回352，弹匣仍为19、备弹仍为90，副武器备弹45也未改变。四张原生截图与各阶段只读playerState采样一并保存，画面已核对。

换枪后的centity有效位筛选曾漏报画面中仍可见的地面枪，该筛选不作为交换成功的单独依据；回取结论来自截图、原生提示、实际选枪及弹药池对照。声音未作听感验收，也不以事件编号或模型数据代替声音结论。

`native-field-codecs.c`保留本版`-78`分派到`0x5a313d`直接读32位值的消费者；错误的短角度布局会使后续玻璃尾部读取溢出，出现`156 -1`，截图见`native-fatal.png`。当前角度编码和混合实体顺序均有独立回归。协议、服务端、严格Clippy及真实采集资源集成检查分别保留结果。诊断客户端已停止，临时配置和证书已恢复，原始客户端与账号数据未修改。

## 证据与复现

本版字段、ITEM组合模型和cursorHint消费者见 `fixtures/protocol/weapon-drops-3.3.3.3.json`。原始汇编、字段解码、`captured-gameplay-proof.json`及`native-proof.json`保存在本机忽略目录，不包含于分发产物。

```sh
sh tools/cargo.sh test --locked -p codol-server \
  captured_shooting_death_drop_and_use_exchange_preserve_real_weapon_identity \
  -- --ignored --nocapture
```

该测试要求本机已准备的真实地图和隔离账号。服务端验证和原生画面分别记录；原生掉枪、F交换和弹药保留已有验收，完整声音及特殊武器仍在[复核清单](REVIEW_PENDING.md)。
