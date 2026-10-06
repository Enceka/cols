# 本地购买、币种映射与武器等级

更新：2026-10-06。当前接续入口。创建房间的玩家等级锁已修复（7353934），详情见 [TEST_ACCOUNT_PROGRESS.md](TEST_ACCOUNT_PROGRESS.md)。真实房间服务仍未实现。

## 当前范围

已接入永久单件装备/外观购买：93/131、80/154，以及购买后的 80/203 库存、80/130 余额刷新。服务端按目标 SKU 价格扣款，价格不取自请求。余额与发放记录同存 wallet.json，一次原子替换；同一服务实例内的并发购买串行处理。拒绝重复购买永久物品、余额不足、错支付方式、未知 SKU、代购和未支持参数。新存储使用 payment="cp"/"gp"，兼容此前实验的 numeric currency=1/2 支付标签；旧记录不丢失。

准备工具产生 4748 个永久单件武器/附件/角色/外观报价。没有实现消耗品、限时物品、礼包、赠送、优惠券或全部商城业务；80/70、80/107 仍返回空目录，界面展示的是客户端内置的商品资源。

**重要纠正**：旧 AlphaBuild 和目标 native GetCP/GetGP 名称不能直接当成这个中文界面的币种语义。用户反馈且截图确认：金色图标是 GP，蓝色 C 图标是 CP。目标 UI 中 wire currency **1=GP、2=CP**。之前的 1=CP、2=GP 注释是错误的，导致视觉上扣反币种。

- 80/230 只能消费第一条余额（0x4a5680），现在返回 GP：currency=1。
- 80/130 同样返回 GP：currency=1。
- 93/1 返回 CP：currency=2。
- 购买请求 80/154 对应 GP 报价，93/131 对应 CP 报价。逻辑支付种类与 wire currency ID 分离，避免再次混淆。
- client-1791284823428/ui-1791284894600.png 及只读状态证明：金色 GP=999500，蓝色 CP=96260，库存 15 件；已购买物品跨重启保留。

## 目标资源与协议证据

StringTable mp/coMarketplaceItems.csv 包含物品 ID/名称，但不含完整 SKU 价格。目标还有 type=45 的 marketplacedata，类型表基址 0xce0ea0。运行时已处理成独立数组，部分原资产条目标记未加载；不绕过检查读取未加载表。

只读探针依据 0x5840a0、0x7072f0、0x4a6550 提取运行时数据：

| 数据 | 指针 / 数量 | 步长 |
| --- | --- | --- |
| SKU | 0x02dbb288 / 0x02dbb28c | 24 字节 |
| Product | 0x02dbb270 / 0x02dbb274 | 24 字节 |

SKU 前 7 个 UInt16 字段为 ID、flags、productID、CP 价格、GP 价格、duration、待定字段；+16/+20 是起止时间。反混淆使用目标读指令相同的 XOR/rotate，过程掩码不导出。duration 由 AS_GetDurationFromSKU 0x5bc910→0x707420 返回 +10 字段。enabled 检查见 0x7074e0，CP 路由见 0x707480。产品的 itemCount 在 +10、UInt16 物品数组指针在 +12；id 在 +0。

提取 client-1791282425621/client-assets-1791282482755.json 含 12350 条 SKU（含零哨兵）、10057 条产品。准备工具保守选择 flags=5、零 duration/起止时间/其他未恢复标志、产品头附加字段为零、单件且在目标物品表内的报价，并限制物品类别。完整资源和生成 shop.json 留在忽略目录。

两个已核对例子：SKU 10073 → item 10183（M4 血红迷彩），2600 GP；SKU 14629 → item 20659（M4 消音器），280 CP。不能将 SKU ID 当成物品 ID。

请求序列化器：93/131 为 0xa4c0b0→0xa64ca0，80/154 为 0xa467e0→0xa45b40。当前只支持单 SKU、数量 1、自购、空 context/metadata、无需 receipt 的真实观测形状；不把任意购买报文当作成功。

购买刷新：80/203 builder=0xa468c0，游戏调用 0x4a6a9f，返回独立 inventory 记录，每条带账户身份；80/130 builder=0xa461d0，返回 UChar+UInt32 余额。80/230 内嵌库存则继承外层身份，两种编码不能混用。

## 动态进展

- client-1791283202415：3 次 CP 购买已扣款、发放，库存 8→11。但随后 80/203、80/130 尚未实现，客户端准确提示“购买成功但刷新玩家仓库时发生错误”。不能把这次当完整购买通过。
- client-1791283928163：补齐刷新后，packet-041 购买成功，042 返回 12 条库存，043 返回余额成功；随后正常回到仓库。重登读到此前 11 件库存与余额，证明前次购买已持久化。
- 用户继续测试两种支付时指出视觉扣反、无法切回 GP，已纠正 wire 币种映射。示例购买历史保留在本地 wallet.json，禁止重置以掩盖错误。
- 当前仍需完成：修正映射后的逐币扣款核对；M9 扩容弹匣的 GP 武器等级限制。

## M9 扩容弹匣的单向切换

client-1791284823428/ui-1791284894600.png：扩容弹匣可选 CP 880 / GP 2300，但卡片标记锁 20；M9 武器等级仍为 1。玩家 rank=70 及 uf_prestige_rank=69 只解决玩家等级门槛。下一步核对武器等级/经验/解锁值，再提升本地测试武器，不能把此限制误判成钱包余额不足。

## 使用与验证

```sh
python3 tools/client_state.py --build
python3 tools/client_state.py --assets --experiment RUN/experiment.json
python3 tools/prepare_shop.py --assets ASSETS_JSON --output NEW_SHOP_JSON
# 将已验证的新 shop.json 放入服务数据目录；生成器拒绝覆盖现有文件。
sh tools/check.sh
```

当前通过 51 项 Rust、13 项 Python 测试及 fmt/Clippy。覆盖真实请求标签和截断、商品关联、限时过滤、原子扣款/发放、并发重复请求只成交一次、重启读回、错币种/透支拒绝、身份隔离与刷新接口。
