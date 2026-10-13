# 训练伤害与世界弹着

更新：2026-10-08，目标客户端3.3.3.3。运行时保持纯Rust。

真实地图油桶与车辆的权威伤害、销毁／残骸和连锁已接入，资源与实机范围见[地图动态物件](MAP_PROPS.md)。下述GENERAL实体19仍是独立训练靶，不承载地图物件状态。

## 已实现与实机证据

- 玩家模拟只输出已通过弹药、冷却、输入去重和交互状态检查的射击，并保留开枪时的武器编号。无弹药、菜单捕获、换弹与切枪期间不产生额外伤害。
- 普通子弹从玩家眼位按输入视角射出，世界射线长度8192，shot mask为本版`0x0280e831`（当前`0x55a4c3`），不使用移动胶囊或移动SKIN。加载器保留移动与射击掩码的并集；各查询仍用自己的掩码。
- 第一距离段内应用组合WeaponDef的真实伤害：伤害`+0xbb4`，第一段距离`+0xbcc`。训练靶使用实采模型边界和同一模型scale，世界遮挡阻止穿墙扣血。射击时间去重与生命周期时钟独立，避免正常落后于快照的usercmd被误判为无效。
- 靶子使用GENERAL实体19、模型156 `com_barrel_russian_fuel_dirt`，100HP、3秒重生。死亡帧显式移除，重生继承固定全基线。可靠文本显示剩余HP和销毁/重生提示。
- 每次有效射击都检查世界弹着，包括打偏靶子和靶子死亡后的射击。活靶挡住后方世界，即使该距离的伤害曲线尚未支持，也不在其后方画弹孔。
- 世界弹着使用ET_EVENTS22 + EV_BULLET_HIT74，即eType96。命中位置、法向编号、当前武器、物理材质和世界命中实体2046进入快照。事件编号轮换20..2045，保持200ms跨帧发送，最多32个活动事件；全基线不含事件，到期省略即移除。
- Brush每个侧面及六个轴向面的真实材质已由只读探针导出；raycast取实际入口面的surface flags。地形同样保留材质。类型为`(flags >> 20) & 31`，仅物理类型0..30有效。
- 用户确认胶囊移动明显顺畅，基础声音已有，视角可控；声音仍未逐项穷尽验收。

实机运行`client-1791436001916`中，探针确认靶子客户端原点为`[-376,-800,-133]`、scale2、model156、DObj slot3。截图`ui-1791436323470.png`可见坦克旁的红黄色训练桶，`ui-1791436228793.png`可见命中点的弹着火花。

服务端记录了多轮`67 → 34 → 1 → 0`扣血。用户明确确认“能扣血、消失并重生”。当前AK117与副武器的弹着事件均被客户端处理；位置、材质、武器编号和一次性latch可由`--bullet-impacts`只读采集。

## 无弹孔的实际原因

GENERAL/event字段2是ES+0x18的X，字段3是ES+0x1c的Y；PLAYER表则先Y再X。此前误用PLAYER顺序，将服务器靶位`[-312,-800,-115]`显示在客户端`[-800,-312,-115]`；世界弹着也被交换到远处。当前`0x412ce0`轨迹消费者和客户端解密原点均确认这一点，已修正两个实体编码器，玩家移动坐标没有改动。

此前Brush没有采材质，统一surface0；实采当前impact FX默认列0为空。即使客户端收到并处理事件，也没有FX可生成。现在采集side+4的u16材质索引、Brush+0x0c..0x16的轴向材质，并读取材质记录+4的surface flags。导出顺序显式映射为Rust的`+X,-X,+Y,-Y,+Z,-Z`。

当前AK117的WeaponDef+0xd0c为4，选择FX row4；副武器值1选择row0。正确物理材质的已载FX包含元素类型13，这是本版原生mark生成元素。探针记录各FX名称与元素类型，以及原生mark开关门槛，便于区分事件编码、资源选择和客户端图形设置。

只读探针还确认客户端`fx_marks=0`，会阻止持久弹孔，同时允许弹着粒子。启动参数`+set fx_marks 1`在本版后续初始化后仍变为0。对于显式启用训练靶的连接，服务器等首个快照ACK后发送一次本版原生可靠设置消息`s fx_marks "1"`，经`0x57d560 → 0x57b690 → 0x634500`设置；不改客户端内存或二进制。

运行`client-1791438122779`已确认该消息实际生效：客户端`fx_marks=1`，其余两个原生mark抑制门槛均为0，世界弹着事件已执行。用户随后射击岩壁或水泥墙，明确确认“能看到留下的弹孔”；持久弹孔显示完成实机验收，不仅是粒子火花。

sky4、noimpact16、nomarks32、nodraw128不输出该弹着事件；几何仍阻挡子弹。尚未恢复仅抑制decal而保留声音/粒子的事件标志，nomarks暂保守抑制整条弹着事件。

## 当前版本协议依据

- GENERAL表`0xc525a8`，61字段，原始表SHA256 `9b789d51c60f89f2df47698952f902740fe611182d4b95b142c96b04f221d0c7`。
- ET_EVENTS表`0xc52888`，61字段，SHA256 `7fa9b797e8dfadcc4ad074f72fa551045fd1e460103517b83be80ea3ffdae84a`。
- eventParm字段5、codec-93，EV74直接8位池编码，无通用非零门；命中实体字段22、codec-96，world2046用单独bit1。
- `0x513970`对每个事件实体只处理一次，固定ID连续重用会丢失效果。`0x561710`消费位置和法向，`0x561220`按注册武器定义选择材质FX；`0x4d6370`处理type13并进入原生marks路径。
- 法向表`0xcda330`实采162方向，最大点积搜索编码；来源表与完整哈希保存在`fixtures/protocol/bullet-impact-3.3.3.3.json`。GENERAL字段fixture位于`fixtures/protocol/entity-state-fields-3.3.3.3.json`。

## 重现

只读采集地图碰撞与靶模型后，用纯Rust准备器生成新配置，不覆盖已有文件：

```sh
target/debug/codol-server prepare-training-target \
  --game-state research-output/training-impact-material-state.json \
  --model-capture research-output/gamehub/client-1791435039734/training-target-1791435228504.json \
  --distance 96 --scale 2 --output research-output/new-training-state.json

python3 tools/gamehub_run.py client --seconds 900 \
  --prefix-name wine10-loadout-20261007 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun --rust-game \
  --local-services --trust-lab-cert --quiet-network \
  --game-state research-output/new-training-state.json --snapshot-probe 18000
```

从启动输出取得本次experiment.json路径；探针支持`--collision`、`--training-target`、`--bullet-impacts`。准备器检查客户端身份、地图CRC、完整shot mask、真实模型边界、地面与出生点到靶子的视线。运行配置、资源捕获、账号资料、证书和密钥均保持在忽略目录，不提交Git。

回归已通过189项Rust测试和30项Python测试，格式与Clippy通过。入口为`RUST_TEST_THREADS=1 sh tools/check.sh`，包括协议独立解码、八种位池对齐、射线/材质、射击授权、伤害/生命周期以及Python工具测试。实际usercmd捕获诊断仍是显式ignored测试。

## 尚未实现

此靶子是静态道具，不是AI敌人。长距离伤害曲线、穿透、散弹、爆炸伤害、骨骼命中、中心命中标记、敌人行为和共享多人世界仍待实现。商城价格与购买按用户要求暂缓。


## 2026-10-10 原生房间换边后的玩家射击

以上“尚未实现”为早期静态靶阶段记录；后续6v6、距离伤害、散弹、骨骼命中和命中HUD进展另见BOT/WEAPON文档。新版参考文档审阅暴露玩家TDM射击仍固定使用阵营2的遗漏，现改为`config.teams[0]`。完整射线回归覆盖两种玩家阵营、队友挡弹无伤、敌人两次中弹死亡、正确队伍加分；不再仅通过直接调用扣血函数验证团队关系。跨地图反馈材质、致命红色反馈与本地声音尚待继续，不将旧Afghan的中央反馈当作三图均完成。


## 2026-10-10 白色命中与红色击杀反馈

本版原生HUD数组已支持Hit/Kill两种材质、颜色与1000/600ms淡出；TDM以实际扣血返回的结果驱动，出生保护和无效命中不再显示成功反馈。声音使用当前sound bank的空闲槽动态登记`MP_hit_alert`，可靠命令`p <relative-slot>`，基址3107。

当前三图均由各自NCS捕获导出`feedback_resources`，旧版单Afghan CRC硬编码已撤掉。三份捕获哈希和消费者地址见`fixtures/protocol/damage-feedback-resources-3.3.3.3.json`。编码测试按独立实采字段表对八种bit-pool对齐分别解码白/红、时长、alpha；状态测试覆盖替换、清空、时钟回绕、声音只取一次；射击回归覆盖两阵营、出生保护和多目标致命优先。

Afghan实机已验证HUD消费、红色标识和再次普通命中的白色恢复；资源表确认命中声加载。尚未把声音的听感或另外两图逐图视觉标作已验收，详细证据见`REFERENCE_ALIGNMENT.md`最新节。


## 2026-10-10 队友死亡定位

已接原生HUD type13与本图`waypoint_dead`：仅同队队友，固定死亡点z+54，不绑定尸体槽；24×24、白色保留贴图红色、不启用边缘箭头。十秒上限，重生/断开/换队/结束提前清理。实机确认骷髅显示和约三秒Bot重生后的消失。

本轮纠正HUD分组：死亡定位current=PS+0xa78，命中反馈archival=PS+0x1ed0；网络先写archival后写current。此前命中探针历史记录来自另一个组，旧数据不能用于证明当前归档语义。完整回放仍未实现。详见`REFERENCE_ALIGNMENT.md`最新节及`death-waypoint-hud-3.3.3.3.json`。

## 2026-10-10 个人与最终击杀回放

现已增加独立的权威历史与回放状态机，接通本版原生进入/退出、倒计时、尸体相机、历史武器/生命/命中反馈及最终回放后结算。用户报告的顶栏无数字已修正并有视觉证据。名片需要另外发送`L <killer> 7`刷新原生缓存；F键实采为0x08，已纠正原0x20假设。完整证据、检查和尚待验收范围见`KILLCAM_PROGRESS.md`。

前三名展示单列为下一项，尚未实现，不能由最终击杀或当前结算页面推定完成。
