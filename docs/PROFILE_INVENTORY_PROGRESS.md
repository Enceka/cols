# 目标档案定义、私有档案与初始库存

更新：2026-10-06。后续工作见 [测试账号与功能限制](TEST_ACCOUNT_PROGRESS.md)。用户已确认仓库可用；等级和余额显示后，又反馈创建房间的等级限制及购买失败，正在继续排查。前一阶段见 [ONBOARDING_PROGRESS.md](ONBOARDING_PROGRESS.md)。

## 已验证结果与剩余边界

- 从运行中的原版 3.3.3.3 只读提取 public/private 定义和物品表，未调用游戏内部函数、注入或修改程序。
- 私有档案 8/2 已返回有效记录。客户端读到本地测试设置的 `unlockFlagsNew[141]=1`，但此 CQB 标志本身不会退出新玩家引导。
- 80/230 已返回 8 件初始库存。实际客户端解析后的记录数为 8，首件物品 ID=792、数量=1。
- 将**已核对的单个公共档案位** `uiNotificationFlags[87]`（onboard_complete）设置后，进入仓库/附件界面，显示 M4A1/SA80-LSW 模型、属性、等级和附件。用户确认仓库可用。
- 这不是完成训练对局的证据。账号是为大厅调试准备的本地测试档案；真实对局、购买、完整目录与奖励、所有装备权限验证仍未实现。
- 公开/私有档案和初始库存保存在本地数据目录，重登已再次加载。受控“切换附件→上传→重登”的独立闭环未在本轮做完；用户已把下一步重点转向等级和本地房间。

当前测试通过 **46 项 Rust、11 项 Python**，包括独立私有档案向量、库存记录向量、身份隔离、坏 CRC/投影拒绝、解压预算、布尔位单位及单标志修改边界。

## 真实实验

目录均在 research-output/gamehub：

| 实验 | 结果 |
| --- | --- |
| client-1791272989896 | 首次完整定义与物品表提取，输出 client-assets-1791273009430.json |
| client-1791273925147 | 提取当前 public/private 内存缓冲；private 是 CRC 正确的全零默认档案；public 部分投影字段已被内存混淆，不能直接保存 |
| client-1791274676960 | Rust 首次发送 private_profile 成功记录 |
| client-1791275093291 | 只读状态证明 CQB flag=1、onboardCharacterID=792；引导仍在 |
| client-1791276070856 | 客户端解析 8 条库存，第一条为角色 792×1；引导/训练请求仍会出现忙碌提示 |
| client-1791276871823 | 等待 UI 资源加载后，提取到 uiNotifications.csv：126 行、11 列，onboard_complete 索引 87 |
| client-1791277210564 | 使用仅修改第 87 位的测试档案；截图 ui-1791277266765.png 为 M4A1 仓库附件界面 |
| client-1791278063344 | 再次启动仍进入仓库；截图 ui-1791278109558.png 为 SA80-LSW 附件界面；270 秒由工具结束 |

所有实验使用标记过的副本与独立 Wine prefix。临时 CA 恢复，原程序保持不变。截图和完整资源/档案提取留在忽略目录。

## 目标定义与字段映射

| 定义 | version | formatChecksum | 数据区 size | 叶字段 | 投影字段 |
| --- | ---: | --- | ---: | ---: | ---: |
| mp/playerdata.def | 233 | 0xe32d50d6 | 14426 | 3414 | 125 Int32 |
| mp/playersecuredata.def | 19 | 0x8a317d69 | 13739 | 9498 | 34 Int32 + 2 Float32 |

**不要再使用 AlphaBuild 的 220/11 定义。** 两份目标定义展开后叶索引完整覆盖；public 的 125 个投影值与真实上传逐项相符。private 的 Float32 投影为 kills/wins，wire tag=13；Int32 为 tag=7。

资源查找依据：哈希函数 VA 0x0049e170，以资产类型为初值，ASCII 小写并将反斜杠归一为斜杠，逐字符 `h=h*31+c`，最后 `%80000`。桶表 VA 0x01bc9f20，条目表 VA 0x01fc4e28、步长 0x1c；type 在 +0，header 指针 +8，hash 链下项 +0x14。名称 getter 0x00483d20。只读取已加载、类型/名称匹配的条目。

StructuredDataDefSet 为 name/count/defs 三个 dword；定义为 0x3c 字节，包含独立叶偏移表。`tools/schema_fields.py` 校验叶覆盖和边界并展开字段：非 bool 的快照偏移为 offsets[leaf]+4；bool 偏移为 4+offsets[leaf]//8、位号 offsets[leaf]%8。

**发现了真实 public 偏移重叠**：例如 weaponClasses[35].name 与 aarScoreboard[0].netcafe_privileges 在 0x24d6 附近重叠。原因尚待确认，因此解析器默认保留原始映射，可用 reject_overlaps 严格检查；不编辑这些重叠区域。private 定义没有发现重叠。本轮只处理有独立验证的字段。

部分 public 投影字段在运行内存中为混淆值：直接抓取 public 缓冲会得到 CRC 不一致、看似巨大的武器 ID。真正 8/3 上传已经由客户端还原；应使用已保存的 public.bin 作为可编辑来源。默认 private 缓冲全零且 CRC 正确，才被允许用作初始化来源。

观察到的档案缓冲头保留为零，并非直接写入定义的 version/checksum。定义版本用于布局校验，不能未经验证便把 233/19 写到快照 +4。当前协议路径依靠 tagged 格式、CRC 和投影恢复，生成器保留原来的头部字节。

## 私有档案与引导状态

`prepare-private-profile` 仅接受 CRC 正确、数据区全零的当前客户端默认 private 快照，拒绝重置已有进度，拒绝覆盖文件。设置 rank=1、UID 高低字，并可选设置 CQB flag；这一步已由客户端显示等级 1 交叉确认。

关键偏移均为含 CRC 的 32768 字节快照偏移：experience=4844、rank=4887、rankVersion=4891、userIDHigh=4943、userIDLow=4947、unlockFlagsNew[141]=8577（Int16）。`uf_mp_cqbtraining=141` 来自本版本 unlockFlags.csv；读取链 0x005b3430→0x004a4ca0。

真正的引导完成条件还使用 `onboard_complete`：0x005c6350 从 mp/uiNotifications.csv 的列 0/1 查找索引，0x005953a0 从 uiNotificationFlags 读取；0x005b3543/0x005c36ff/0x0069866c 都引用该条件。提取索引为 87，对应快照 offset=4390、bit=0。

`tools/prepare_lobby_profile.py` 生成新的候选文件，不动原文件。实测候选只改变此一位和 CRC，125 个投影值不变，并通过 Rust 校验。应用前备份在 research-output/public-before-lobby-flag.bin；准备记录为 public-lobby-preparation.json。后续客户端正常化保存可能改变压缩字节，不能用候选文件覆盖新的保存结果。

## 初始库存

物品表 mp/coMarketplaceItems.csv 实际为 6772×12。`prepare_inventory.py` 从**已保存 public.bin 的当前角色和第一套核心配装**选取 ID，再逐一验证它们存在于目标物品表，生成 title/UID 绑定的 inventory.json。当前 8 件：

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

80/230 的 inventory 内嵌记录解码器 VA 0x00a4b850，构造器默认值 0x00a4b4e0：继承外层账户；UInt32 itemID/quantity/XP，Blob，UInt32、Int64、UInt16、UInt32、UChar。当前 quantity=1、XP=0、Blob 为空，后五项采用构造器默认值 0xffffffff、i64::MAX、0、0xffffffff、0。额外字段完整业务语义仍待恢复；不声称租赁/到期/交易系统已实现。

独立 Python 合成向量验证顺序和标签；实际 SDK 结果计数地址 0x07bdc9b0、记录指针 0x07bdc9b4，第一项 +0x30/+0x34 读到 792/1。原商城出售目录仍为空，客户端能预览其他资源不等于服务端已实现它们的购买或授予。

## 工具与运行

```sh
python3 tools/client_state.py --build
# 在客户端 UI 加载后执行；过早提取可能标记表为尚未加载。
python3 tools/client_state.py --assets --experiment EXPERIMENT_JSON
python3 tools/client_state.py --profiles --experiment EXPERIMENT_JSON

target/debug/codol-server prepare-private-profile --assets ASSETS_JSON \
  --snapshot PROFILES_JSON --service-data-dir client-work/server-data --training-complete
python3 tools/prepare_inventory.py --assets ASSETS_JSON \
  --public-profile PUBLIC_BIN --service-data-dir client-work/server-data
python3 tools/prepare_lobby_profile.py --assets ASSETS_JSON \
  --public-profile PUBLIC_BIN --output NEW_CANDIDATE_BIN

python3 tools/gamehub_run.py client --seconds 270 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun \
  --local-services --trust-lab-cert --quiet-network
```

现有账号已经准备好，不要重新初始化或覆盖。新 --quiet-network 只关闭 Wine winsock 噪声，Rust 报文捕获仍开启。提取器始终只读，所有完整资源和真实档案输出不得加入 Git。

## 用户最新方向

用户已确认仓库可用，要求提高测试等级，并把下一步转向本地房间/人机。目标 rankTable.csv 已提取：maxrank=69，表版本=4，显示等级为 1..70；最高行起始累计经验 3582550。等级调整需先核对目标经验字段的实际单位，再备份和更新 private.bin，不应再次调用拒绝覆盖的初始化命令。

下一步先完成高等级测试账号，再观察房间创建/模式地图选择产生的请求，区分房间控制协议、21/5 对局分配、本地主机残留与真正 Rust 游戏服。用户未要求发布或推送；每个验证阶段及时 Git commit。
