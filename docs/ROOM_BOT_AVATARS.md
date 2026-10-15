# 房间 Bot 角色展示

固定客户端为 CODOL 3.3.3.3，EXE SHA-256：
`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。
下文地址均为本版 PE 首选映像 VA。

## 当前行为

模拟 Bot 使用原生 type18 装配报文发布角色包、服装、主副武器和配件。
房主确认成员身份及实际队伍后，服务端按本图职业装配发送；换队、选图、取消准备和房主外观刷新会更新展示。
相同装配去重，前端重建时强制重发；回送房主的原生消息先于 Bot 准备状态和装配。
已有名单及局内模型同步各自保留，房间展示使用独立的物品 ID。

本机23份正常地图准备包含 `match_config.roster[*].room_loadouts`。
各 Bot 保留自己的职业、角色和枪械；双阵营装配按游戏队伍顺序 Axis／Allies 保存，房间队伍1／2使用相反索引。
正常准备为7500分、600000ms，旧准备备份及逐图哈希保存在忽略目录。
缺少房间物品数据的旧准备仍能加载，但不生成对应 Bot 装配，需重新准备。

服务端Bot与真人type18／自身type28配装同步均已接入；双真人房间展示和共享对局核心流程有独立验收，见[多人](MULTICLIENT_ACCEPTANCE.md)。Bot专题的更多待机动作、地图和首帧加载仍单独复核。

## 模块归属

| 职责 | 当前入口 |
| --- | --- |
| type18 格式和字段验证 | [protocol/room/messages.rs](../crates/protocol/src/room/messages.rs) 的 `Loadout` |
| Bot 装配选择、去重和发布 | [backend/rooms/bots.rs](../crates/server/src/backend/rooms/bots.rs) 的 `Policy`、`Channel::sync_bot_loadouts` |
| 成员确认、换队和前端刷新触发 | [backend/rooms/mod.rs](../crates/server/src/backend/rooms/mod.rs) 的房间 handler |
| 双阵营准备字段 | [game/ai/loadout.rs](../crates/server/src/game/ai/loadout.rs) 的 `BotLoadout.room_loadouts` |
| 模式准备验证 | [game/modes/tdm/config.rs](../crates/server/src/game/modes/tdm/config.rs) |
| 地图目录到房间策略的装配 | [game/session/catalog.rs](../crates/server/src/game/session/catalog.rs)、[transport/game/server/mod.rs](../crates/server/src/transport/game/server/mod.rs) |
| 本版物品准备 | [prepare_room_bot_loadouts.py](../tools/prepare_room_bot_loadouts.py) |
| 只读成员、外观缓存和展示席位诊断 | [ReadClientState.cs](../tools/ReadClientState.cs) |

房间业务不依赖接入层或运行时世界；接入装配从已验证地图准备取出无凭据的房间物品数据。
嵌入 fixture 路径以 `CARGO_MANIFEST_DIR` 为基准。

## 资源准备

准备器从本版角色变体索引反查服装物品，匹配性别、肤色及默认／允许部件一致的角色包。
武器物品必须同时匹配本图 arsenal 和 marketplace 身份。
模糊角色包、缺失身体部件、错误阵营、缺少对应模型或武器不一致均拒绝；失败不替换输入状态。
当前 Bot 职业主武器没有显式配件或副武器，前端按原生武器目录补齐默认配件。

角色包 `cha`、商城物品 ID、clientState 变体索引、gamestate 模型槽和局内武器注册索引具有不同含义。
房间数据只能使用真实物品 ID；客户端会核对角色包所属部件。
准备命令使用紧凑 JSON，保留准备文件65536字节上限。

```sh
python3 tools/prepare_room_bot_loadouts.py \
  --game-state /path/to/prepared-map.json \
  --appearance /path/to/appearance-catalog.json \
  --output /path/to/new-prepared-map.json
```

`prepare_bot_roster.py` 和 `prepare_tdm_map.py` 在提供原生角色捕获及外观目录时同步准备房间装配。
运行配置和资源目录仍见 [本地游玩](LOCAL_PLAY.md)。

## 原生协议与展示消费

裸 type18 正文649位、82字节，尾部7位为零填充；batch 长度头增加4字节。
外层沿用房间 untyped bit stream 和 SDK event7；发送者身份必须是已接纳的对应 Bot。
正文不携带 UID，接收器按 channel 发送者补齐身份。

| 顺序 | 内容 | 位数 | 装配内存偏移 |
| --- | --- | ---: | --- |
| 1 | untyped 标记0、kind18 | 1+8 | 非装配字段 |
| 2 | 原生 flag，当前默认0 | 8 | `+0x08` |
| 3 | 房间队伍 | 8 | `+0x0c` |
| 4 | 12个角色／服装物品 | 12×16 | `+0x10..+0x26` |
| 5 | 主、副武器及各六个显式配件 | 2×7×16 | `+0x28..+0x42` |
| 6 | 六个装备槽 | 6×16 | `+0x44..+0x4e` |
| 7 | 三个技能槽 | 3×16 | `+0x50..+0x54` |
| 8 | 三个奖励槽 | 3×16 | `+0x56..+0x5a` |
| 9 | 尾部物品槽 | 16 | `+0x5c` |

角色槽顺序为 `cha,fpa,boo,fja,han,hea,ves,heg,elp,knp,pat,eye`。
服装使用原生16位 short，武器等物品使用16位 unsigned short。
flag 的完整语义和尾部槽的具体类别仍待核对。

| 原生入口 | 已核对行为 |
| --- | --- |
| `0x6df130 / 0x6df360` | type18 写入／读取 |
| `0x6e8f10` case `0x12` | 按发送者查成员，将装配写入 `member+0x100`，通知 listener `+0x1c` |
| `0x6dc2d0` | 主武器物品解析 WeaponDef `+0x480` 的类别，写入装配 `+0x60` |
| `0x692150 / 0x68ec60` | 更新 UID装配缓存，调用 `cppSetCharacterAvatarSlots` |
| `0x68e9c0 / 0x56dc50` | 筛选当前展示队伍，生成最多9个席位 |
| `0x56de90` | 填入角色包及其所属部件 |
| `0x568ba0 / 0x568a80` | 主武器、显式及默认配件组装前端枪模 |
| `0x567860 → 0x603eb0` | 队伍场景进入原生展示动画系统 |
| `0x56cb80` | 房间场景 mode3 使用 `team_lobby` 相机预设 |

type18 没有 legs／torso 动画 ID；客户端根据装备和武器类别选择房间动作，不能套用局内动画编号。
12人 war房间按当前队伍展示6个人物，另一队使用独立视图。

## 验证与证据边界

证据保存在本机忽略目录，没有账号凭据进入公开 fixture。

| 验证 | 证据与范围 |
| --- | --- |
| 本版报文编码 | 四份保留原生房主 type18 由 Rust 重编码后逐字节相等，见 `native-encoding-samples.json`／`native-encoding-test.log` |
| 物品及生命周期回归 | 合成fixture验证649位格式、非法字段和截断；房间测试覆盖接纳前不发、实际队伍、去重、选图、移除成员、取消准备和外观重发顺序 |
| 双队伍展示 | ：11只Bot服装12槽、武器14槽及实际队伍与准备逐项一致；12项UID缓存、两队各6个有效席位。完整画面 原生画面证据／原生画面证据 确认各自人物、枪械、站立／坐姿 |
| 选图和开局 | 同会话原生选到杀戮小屋进入state10，短局结算1000:900 |
| 结算返回房间 | ：state10→100:1000结算→原房间及选图箭头往返；11只Bot装配、12项缓存和6个席位恢复，所有成员未准备；`lifecycle-proof.json` 和 原生画面证据 确认返回画面 |
| 旧Bog原生组成 | 本图捕获及角色银行匹配职业组成；正常准备60秒后台模拟通过，11只移动、28次击杀、28次重生，见 `normal-bog-check.json`；不是本图房间画面验收 |

首次切换暗影队视图曾出现空场景，后续画面完整；异步加载的首帧时机尚未验收。
全部待机动作、更多地图房间画面及长期稳定性仍需专项验证。
23图15秒后台检查完成资源加载与移动，但13图未在短窗口产生击杀或重生，整体失败；不作为全地图玩法验收。
客户端EXE哈希保持一致，正常账号资料使用独立诊断副本；`final-proof.json` 记录返回结果和诊断清理。

静态指令在 `native-functions.asm`；对应只读函数导出在
本版静态消费证据、本版静态消费证据、
本版静态消费证据、本版静态消费证据 和 本版静态消费证据。

```sh
python3 tools/check_architecture.py
sh tools/cargo.sh test --locked -p codol-protocol room::messages
sh tools/cargo.sh test --locked -p codol-server backend::rooms
python3 -m unittest discover -s tools/tests -p 'test_prepare_room_bot_loadouts.py' -v
CODOL_ROOM_AVATAR_CAPTURE=/absolute/path/native-encoding-samples.json \
  sh tools/cargo.sh test --locked -p codol-protocol \
  retained_native_owner_loadouts_reencode_identically -- --ignored
```
