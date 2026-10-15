# 2.6.9.7 加载文档对照：3.3.3.3 模型排查

结论：这份资料提供了值得采用的资源排查方法，尤其是公共/商城包中的第一人称几何、模型表、动画与DObj装配。它没有证明当前异常是缺包或第三人称动画，也不能直接提供3.3.3.3的结构偏移或修复参数。

## 本版已复核的事实

新增只读工具 `tools/inspect_resource_lists.py`，只读原始样本每包前2MiB，不加载DLL、不解压或合并补丁。候选偏移0xac必须通过nifs头、ASCII .lst首行和连续路径校验。使用已记录的362项库存核对名称/长度；prefix SHA只校验读取的前缀，不冒充整包或内部块完整性。

```sh
python3 tools/inspect_resource_lists.py --output research-output/ifs-lists-3.3.3.3.json
```

本次362包中识别324份明文清单，得到73839条路径、去重58018条，其中5778个 `main/models/` 名称。没有清单读到设定上限。其余38包未识别到这种明文布局，**不等于空包、损坏或未下载**；路径并集也不是补丁应用后的最终文件系统。文档中的276包、51场景、旧包版本号不能套用本版。

阿富汗当前实验的路径直接对应：

| 虚拟路径 | 3.3.3.3物理包 | 清单中文件偏移 |
| --- | --- | --- |
| `client_shipRetail/chinese/mp_afghan.ff` | `ff_dlc2_mp_afghan_V32.ifs` | 0xc3 |
| `client_shipRetail/chinese/mp_afghan_load.ff` | 同上 | 0xeb |
| `client_shipRetail/chinese/mp_afghan_const.ff` | `lf_dlc2_mp_afghan_const_V37.ifs` | 0xc9 |

M4模型/配件确实跨商城公共包；这支持文档5.2～5.4的排查方向：

| `main/models/` 后的名称 | 包 | 清单中文件偏移 |
| --- | --- | --- |
| `wea_m4_m4_vmgun10` | `lf_init_common_marketplace_2_V19.ifs` | 0x1e46b |
| `wea_m4_m4_lod210` | 同上 | 0x1e44d |
| `wea_m4_m4_lod110` | `lf_init_common_marketplace_4_V21.ifs` | 0x1e8da |
| `wea_m4_m4_lod010` | `lf_init_common_marketplace_5_V21.ifs` | 0x1ea1c |
| `gri_default_m4_vmgun10` | `lf_init_common_marketplace_2_V19.ifs` | 0x17fa0 |
| `sto_default_m4_vmgun10` | `lf_init_common_marketplace_5_V21.ifs` | 0x1d2b5 |

精确字符串 `viewhands_player_tf141`、`viewmodel_m4a1tech` 未在这些外部几何路径中出现，不能由此判定缺失：FF资产名、外部几何名及动画名不是同一命名空间，也有38包未解析。当前画面已经有手臂和部分武器几何，还须核对它们是否是正确资源与骨骼组合。

## 从文档字符串线索定位到目标代码

下表明确区分**文件偏移、字符串VA、代码中立即数操作数位置**；后者不是函数入口。

| 字符串 | 本版文件偏移 | 字符串VA | 操作数位置 |
| --- | --- | --- | --- |
| `mp/coWeapModels/%s.csv` | 0x884c3c | 0xc84c3c | 0x707aa5、0x7081b6 |
| `animlists/%s.animlist` | 0x85bad8 | 0xc5bad8 | 0x40bc6c |
| `BG_TryLoadAnimList` | 0x85baf0 | 0xc5baf0 | 0x40bd11 |
| `R_AddDObjSurfaces` | 0x869ea0 | 0xc69ea0 | 0x4f6e8f |

旧文档提到的 `mp/coWeaponAttachmentTable.csv` 没有在本版EXE中找到相同ASCII串；这只说明该检索没有命中，不能推断本版没有配件表。

已继续反编译本版**函数入口**0x707a50、0x708190，证据保存在忽略目录 `research-output/ghidra/loading-report-model-tables.c`：

- 0x707a50根据记录选择 `mp/coWeapModels/%s.csv`，通过type37的StringTable查找，并从所选行的列2、列3输出模型名称。
- 0x708190查同类模型表的指定行，将列2～5交给0x708000处理装配参数。参数语义仍需追踪，不能仅凭列号命名为已确定的旋转/位移。
- 前序已确认的组合武器函数0x42dc10调用上述函数，并建立模型部分记录。至此“表驱动装配”在3.3.3.3中具有目标函数证据，不只是旧文档的流程推测。

## 当前使用方式

按本版模型表、默认件、DObj／XAnim及实际骨骼映射定位显示问题；资源路径、资产名和动作名不是同一命名空间。静态清单缺少一个名字不能直接证明缺包。

默认件、角色衣袖／手套、稳定ADS及移动已接入，见[持枪模型](VIEWMODEL_PROGRESS.md)、[背包](PLAYER_LOADOUT_PROGRESS.md)与[移动](GAMEPLAY_INPUT_PROGRESS.md)。参考文档只提供方法线索，不能直接提供3.3.3.3偏移或替代本版验收。
