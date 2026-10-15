# 档案定义、私有档案与库存

本版 public v233、private v19 定义及物品表已通过只读提取和原生消费核对。公共档案保存／重登、有效私有档案和仓库操作已验证；测试账号可持有 6772 项物品，独立新玩家可选择基础库存或全物品。模板和身份隔离见[本地新玩家](LOCAL_PLAYER.md)，进度投影见[等级](PROGRESSION_PREPARATION.md)。

## 目标定义与字段映射

| 定义 | version | formatChecksum | 数据区 size | 叶字段 | 投影字段 |
| --- | ---: | --- | ---: | ---: | ---: |
| mp/playerdata.def | 233 | 0xe32d50d6 | 14426 | 3414 | 125 Int32 |
| mp/playersecuredata.def | 19 | 0x8a317d69 | 13739 | 9498 | 34 Int32 + 2 Float32 |

**不要再使用 AlphaBuild 的 220/11 定义。** 两份目标定义展开后叶索引完整覆盖；public 的 125 个投影值与真实上传逐项相符。private 的 Float32 投影为 kills/wins，wire tag=13；Int32 为 tag=7。

资源查找依据：哈希函数 VA 0x0049e170，以资产类型为初值，ASCII 小写并将反斜杠归一为斜杠，逐字符 `h=h*31+c`，最后 `%80000`。桶表 VA 0x01bc9f20，条目表 VA 0x01fc4e28、步长 0x1c；type 在 +0，header 指针 +8，hash 链下项 +0x14。名称 getter 0x00483d20。只读取已加载、类型/名称匹配的条目。

StructuredDataDefSet 为 name/count/defs 三个 dword；定义为 0x3c 字节，包含独立叶偏移表。`tools/schema_fields.py` 校验叶覆盖和边界并展开字段：非 bool 的快照偏移为 offsets[leaf]+4；bool 偏移为 4+offsets[leaf]//8、位号 offsets[leaf]%8。

**发现了真实 public 偏移重叠**：例如 weaponClasses[35].name 与 aarScoreboard[0].netcafe_privileges 在 0x24d6 附近重叠。原因尚待确认，因此解析器默认保留原始映射，可用 reject_overlaps 严格检查；不编辑这些重叠区域。private 定义没有发现重叠。仅处理有独立验证的字段。

部分 public 投影字段在运行内存中为混淆值：直接抓取 public 缓冲会得到 CRC 不一致、看似巨大的武器 ID。真正 8/3 上传已经由客户端还原；应使用已保存的 public.bin 作为可编辑来源。默认 private 缓冲全零且 CRC 正确，才被允许用作初始化来源。

观察到的档案缓冲头保留为零，并非直接写入定义的 version/checksum。定义版本用于布局校验，不能未经验证便把 233/19 写到快照 +4。当前协议路径依靠 tagged 格式、CRC 和投影恢复，生成器保留原来的头部字节。

## 私有档案与引导状态

`prepare-private-profile` 仅接受 CRC 正确、数据区全零的当前客户端默认 private 快照，拒绝重置已有进度，拒绝覆盖文件。设置 rank=1、UID 高低字，并可选设置 CQB flag；这一步已由客户端显示等级 1 交叉确认。

关键偏移均为含 CRC 的 32768 字节快照偏移：experience=4844、rank=4887、rankVersion=4891、userIDHigh=4943、userIDLow=4947、unlockFlagsNew[141]=8577（Int16）。`uf_mp_cqbtraining=141` 来自本版本 unlockFlags.csv；读取链 0x005b3430→0x004a4ca0。

真正的引导完成条件还使用 `onboard_complete`：0x005c6350 从 mp/uiNotifications.csv 的列 0/1 查找索引，0x005953a0 从 uiNotificationFlags 读取；0x005b3543/0x005c36ff/0x0069866c 都引用该条件。提取索引为 87，对应快照 offset=4390、bit=0。

`tools/prepare_lobby_profile.py` 生成新的候选文件，不动原文件。实测候选只改变此一位和 CRC，125 个投影值不变，并通过 Rust 校验。应用候选前备份现有档案；客户端正常化保存后，不使用旧候选覆盖新的保存结果。

## 初始库存

物品表 mp/coMarketplaceItems.csv 实际为 6772×12。`prepare_inventory.py` 从**已保存 public.bin 的当前角色和第一套核心配装**选取 ID，再逐一验证它们存在于目标物品表，生成 title/UID 绑定的 inventory.json。基础模板的 8 件：

| ID | 目标名称 |
| ---: | --- |
| 792 | cha_INFbaseE |
| 10001 | m4_mp |
| 10049 | m9_mp |
| 60001 | semtex_mp |
| 60006 | emp_grenade_mp |
| 20379 | red_normalm4 |
| 20782 | for_gripvtlong2m4 |
| 25684 | red_rmrm9 |

没有把 perks/wildcards/special 中的数字 1 当作物品：这些字段含非物品标量，且有部分旧引用不在目标表中。仅提供已核实核心装备；不是完整默认背包。

80/230 的 inventory 内嵌记录解码器 VA 0x00a4b850，构造器默认值 0x00a4b4e0：继承外层账户；UInt32 itemID/quantity/XP，Blob，UInt32、Int64、UInt16、UInt32、UChar。基础记录 quantity=1，未启用经验时 XP=0、Blob 为空，后五项采用构造器默认值 0xffffffff、i64::MAX、0、0xffffffff、0。额外字段完整业务语义仍待恢复；不声称租赁/到期/交易系统已实现。

独立 Python 合成向量验证顺序和标签；实际 SDK 已读取有效初始库存。商城目录初始化仍使用空列表，客户端能预览其他资源不等于服务端已实现它们的购买或授予。

## 准备工具

```sh
python3 tools/client_state.py --assets --experiment EXPERIMENT_JSON
python3 tools/client_state.py --profiles --experiment EXPERIMENT_JSON
target/release/codol-server prepare-private-profile --assets ASSETS_JSON \
  --snapshot PROFILES_JSON --service-data-dir SERVICE_DATA_DIR --training-complete
python3 tools/prepare_inventory.py --assets ASSETS_JSON \
  --public-profile PUBLIC_BIN --service-data-dir SERVICE_DATA_DIR
python3 tools/prepare_lobby_profile.py --assets ASSETS_JSON \
  --public-profile PUBLIC_BIN --output NEW_CANDIDATE_BIN
```

初始化命令拒绝覆盖已有资料。全物品授予只代表所有权，不代表武器等级、配件／插槽门槛、剧情通关或全部装备效果已实现。已保存公共档案是编辑来源，不能把混淆的内存快照写回账号。
