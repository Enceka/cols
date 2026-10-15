# 本地购买、币种与解锁边界

永久单件购买、库存／余额刷新及跨重启保留已有实现和原生消费证据。商城扩展保持暂缓；武器等级由独立经验服务处理，见[等级](PROGRESSION_PREPARATION.md)。

## 当前范围

已接入永久单件装备/外观购买：93/131、80/154，以及购买后的 80/203 库存、80/130 余额刷新。服务端按目标 SKU 价格扣款，价格不取自请求。余额与发放记录同存 wallet.json，一次原子替换；同一服务实例内的并发购买串行处理。拒绝重复购买永久物品、余额不足、错支付方式、未知 SKU、代购和未支持参数。钱包存储使用 payment="cp"/"gp"，兼容已有 numeric currency=1/2 支付标签；保留既有记录。

准备工具产生 4748 个永久单件武器/附件/角色/外观报价。没有实现消耗品、限时物品、礼包、赠送、优惠券或全部商城业务；80/70、80/107 仍返回空目录，界面展示的是客户端内置的商品资源。

界面金色图标是 GP，蓝色 C 图标是 CP；wire currency **1=GP、2=CP**。逻辑支付种类与协议币种分开处理，不根据旧版本函数名推断。

- 80/230 只能消费第一条余额（0x4a5680），现在返回 GP：currency=1。
- 80/130 同样返回 GP：currency=1。
- 93/1 返回 CP：currency=2。
- 购买请求 80/154 对应 GP 报价，93/131 对应 CP 报价。逻辑支付种类与 wire currency ID 分离，避免再次混淆。

## 目标资源与协议证据

StringTable mp/coMarketplaceItems.csv 包含物品 ID/名称，但不含完整 SKU 价格。目标还有 type=45 的 marketplacedata，类型表基址 0xce0ea0。运行时已处理成独立数组，部分原资产条目标记未加载；不绕过检查读取未加载表。

只读探针依据 0x5840a0、0x7072f0、0x4a6550 提取运行时数据：

| 数据 | 指针 / 数量 | 步长 |
| --- | --- | --- |
| SKU | 0x02dbb288 / 0x02dbb28c | 24 字节 |
| Product | 0x02dbb270 / 0x02dbb274 | 24 字节 |

SKU 前 7 个 UInt16 字段为 ID、flags、productID、CP 价格、GP 价格、duration、待定字段；+16/+20 是起止时间。反混淆使用目标读指令相同的 XOR/rotate，过程掩码不导出。duration 由 AS_GetDurationFromSKU 0x5bc910→0x707420 返回 +10 字段。enabled 检查见 0x7074e0，CP 路由见 0x707480。产品的 itemCount 在 +10、UInt16 物品数组指针在 +12；id 在 +0。

本版只读采集含 12350 条 SKU（含零哨兵）、10057 条产品。准备工具保守选择 flags=5、零 duration/起止时间/其他未恢复标志、产品头附加字段为零、单件且在目标物品表内的报价，并限制物品类别。完整资源和生成 shop.json 留在忽略目录。

两个已核对例子：SKU 10073 → item 10183（M4 血红迷彩），2600 GP；SKU 14629 → item 20659（M4 消音器），280 CP。不能将 SKU ID 当成物品 ID。

请求序列化器：93/131 为 0xa4c0b0→0xa64ca0，80/154 为 0xa467e0→0xa45b40。当前只支持单 SKU、数量 1、自购、空 context/metadata、无需 receipt 的真实观测形状；不把任意购买报文当作成功。

购买刷新：80/203 builder=0xa468c0，游戏调用 0x4a6a9f，返回独立 inventory 记录，每条带账户身份；80/130 builder=0xa461d0，返回 UChar+UInt32 余额。80/230 内嵌库存则继承外层身份，两种编码不能混用。

## 武器等级与购买门槛

人物等级和武器等级分别计算。M9 扩容弹匣的 GP 渠道要求武器显示等级 20；提高人物 rank 或钱包余额不能解除这一条件。配件的其它购买渠道、插槽解锁和物品所有权也不能混为一个门槛。

```sh
python3 tools/client_state.py --assets --experiment EXPERIMENT_JSON
python3 tools/prepare_shop.py --assets ASSETS_JSON --output NEW_SHOP_JSON
```

生成器拒绝覆盖已有文件。准备的 `shop.json` 和钱包保存在本机服务目录；修改已有钱包前保留有效记录，不以重置资料代替迁移。测试覆盖支付校验、并发串行化、扣款／授予原子性和库存刷新；完整商城、逐币体验和解锁组合需分别验收。
