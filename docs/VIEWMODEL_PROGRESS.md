# 第一人称持枪模型与动画绑定

2026-10-07。接续 [加载文档对照](LOADING_2697_REVIEW.md)。本轮确认并修正手臂资源选择：`--viewhands` 默认改用NCS中的 **viewhands_tf141（当前地图ID54）**。旧候选viewhands_player_tf141（ID53）会出现手臂张开、悬空的异常姿态。用户最新截图在仅换手臂的实验中显示手臂回到持枪位置；枪身默认配件与ADS状态仍待完成，不宣称整个武器显示已经正常。

## 模型表与实际装配

新只读模式 `tools/client_state.py --experiment 实验路径 --weapon-models` 输出当前M4模型表、组合模型、兼容配件、DObj模型/骨骼、XAnim叶资源与活动节点。仍校验研究副本标记和EXE哈希，不修改客户端内存或调用游戏函数。

`client-1791365818698/weapon-models-1791365982128.json`：

- `mp/coweapmodels/m4_mp.csv` 已加载，33行7列；第0行选择 `wea_m4_m4_blk` / `wea_m4_m4_blk_vmgun`，其他行主要是涂装/细节覆盖。并非默认配件列表。
- 组合 `** 163 3 0` 的attachment_count=0、模型部分只有枪身。其他m4a1tech变体模型表未加载，不能把资产表中存在名称当作本次已加载。
- 0x42e3b0将WeaponDef嵌入组合记录+0xa0；0x42dc10写入+0x91配件数、+0x92模型数、+0xf68世界模型数组、+0xfac第一人称数组。
- M4兼容配件来自 `0x17f5848+baseId` 的计数和 `0x17f8248+baseId*0x100` 的ushort二元组。`0x17e5848+attachmentId*4` **直接指向名称字符串**，不是再解一次引用的结构体。首个错误探针将ASCII `aco_`当成地址，已纠正；未对游戏做写入。

兼容列表有20项，例如aco_ta31m4、mag_extclipm4、for_gripafgm4、sco_none等，没有得到完整默认握把/枪托清单。不能简单把全部兼容附件一起装上；下一步从默认部件选择逻辑补齐组合输入。

## DObj与骨骼映射的目标依据

| 对象 | 本版来源 |
| --- | --- |
| 每手记录 | 0x557390/0x5587e0：槽选择0x07817bb8，表0x01a19510，xor0x39641402；stride24 |
| DObj模型数组 | 0x644860写DObj+9数量、+0xc0间接指针数组；0x59e610迭代该数组 |
| XModel骨骼 | 0x644860/0x6492a0使用XModel+4数量、+0x2c的ushort脚本字符串ID数组 |
| XAnimParts骨骼 | 0x649320使用Parts+0x1a数量、+0x30的ID数组；旧版+0x1b不是本版骨骼数量 |
| 动画到模型映射 | 0x649320生成32字节位掩码、1字节数量及每骨骼1字节索引；0xfc表示未匹配，结果通过脚本字符串池保存 |
| 活动节点 | 0x64b490：0x0374db08池、stride0x38、next+0xa/children+0xc；0x64c880：时间+0x18、目标权重+0x28、权重+0x2c、速率+0x30 |

证据文件在忽略目录：`dobj-create-current.c`、`reload-button-dobj-models.c`、`xanim-binding-current.c`、`xanim-weight-state.c`、`viewmodel-root-selection.c`。不再使用“扫描DObj头部寻找直接XModel指针”的旧办法。

`client-1791367893375/weapon-models-1791368004165.json`：实际模型为viewhands_player_tf141和wea_m4_m4_blk_vmgun。待机/开火叶动画有71根骨骼，实际映射63根，匹配项的名字无错位；8根未匹配的是下挂武器等额外部件。ADS-down叶有1根骨骼且匹配。`client-1791366776335/weapon-models-1791367494743.json` 中待机节点和ADS-down节点权重均为1，待机已到时间1。这排除了简单的“没有动画资源/没有权重”解释，并不证明网格和动画的初始姿态完全兼容。

两个手臂候选都有66根骨骼，仅比较骨骼名称不能选定正确资源。没有取得证据证明第三人称动画被当成第一人称；实际叶资源为viewmodel_m4a1tech_idle/fire及viewmodel_m4_reload等。

## 单变量对照与修正

`training-alt-hands-state.json` 与之前换弹配置相比仅将viewmodel_index从53改为54，ID仍从同版本NCS资源顺序计算。`client-1791368260738` 中用户截图显示手臂与枪身相对位置明显恢复；`weapon-models-1791368754840.json` 确认实际DObj已换成viewhands_tf141，而枪身仍为wea_m4_m4_blk_vmgun。

准备工具默认选用通过该对照的viewhands_tf141。`--viewhands-model viewhands_player_tf141`保留旧模型作为显式比较选项，不作为默认。测试同时验证两个名字分别解析到各自的资源索引，未硬编码54。19项Python测试通过，只读探针已经重新编译并用于真实进程采集。

主动换弹修复a331f09已由用户明确确认“换弹可以了”。此前空仓自动换弹的验收不再代替主动换弹验收。

下一步：补齐默认枪械部件，然后为ADS维持服务端连续状态；当前固定PS瞄准进度仍会干扰客户端预测动作。移动/碰撞及命中仍不属于已完成能力。


## 默认部件资料接续

用户明确要求复用AlphaBuild逻辑并保持纯Rust，路线见 [ALPHA_RUST_PORT_PLAN.md](ALPHA_RUST_PORT_PLAN.md)。已定位参考 `codol_weapon_compose.c::CODOL_WeaponDeclaredDefaults`：按武器读取默认槽位、再核对部件模型。它的旧版组合名含item/variant对，本版不能直接复制此序列化格式。

`client-1791369851717/weapon-models-1791370636389.json` 在真实对局中成功采集：本版数据库存在 `mp/coweapitemswea.csv`、各group的部件数据与模型表名称；但对应StringTable在当前资产就绪门槛（entry+0x11 & 5）未通过，因此探针报告未加载并未解引用其表体。目标0x49e7d0同样检查此门槛并在需要时触发加载。存在名称与本次可读表体不是同一件事，不以空表代替默认配方。

注册表中for_default、muz_default、mag_default、gri_default、sto_default、iro_default、rai_default等同名项重复出现于多个ID。下一步必须按武器及槽位关联，不能选择第一个同名ID，也不能把所有兼容配件全加进去。参考算法提供表驱动原则，具体行、注册ID及本版替代资源仍需恢复。

两次探针在大厅而非地图运行时失败（未形成有效装配采集），已增加connection_state=10的前置检查，避免把大厅对象当作对局DObj。新模式继续只读；UI进入地图后的上述采集成功。配方读取与真正发送完整组合尚未完成。
