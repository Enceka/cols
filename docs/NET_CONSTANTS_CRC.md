# 3.3.3.3 网络常量 CRC

2026-10-07。目标 EXE SHA-256：`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。实现见 `tools/ClientAssets.cs` 与 `tools/prepare_game_state.py`；只读采集，不修改客户端、不关闭校验。

## 算法与静态依据

1. `0x5855d0` 从已注册 network dvar 名称表生成公共 seed。数量位于 `0x02dfaba4`，指针表 `0x02dfaba8`，dvar 的首字段是名称指针。按注册顺序对原始字符串字节做标准 CRC32，初值 0，**不包含 NUL**。本次 193 个名称独立复算为 `0xcdb31851`，与 `0x02df9d38` 一致。
2. `0x5850b0` 先调用动态常量构造器 `0x5857b0`，然后将 current CRC **覆盖回公共 seed**，再枚举 type44 资源。动态 category 18/19/20 先加入查找链；不能把它们在 seed 之前产生的 CRC 再算一次。
3. 比较器 `0x585000` 依次按 source、category、localized 标志、名称（ASCII 不区分大小写）排序。标志：名称长度 >12，且从第 8 字节开始匹配当前语言的 `language_` 前缀（`0x4a03b0`）；本次 `chi_`。标志 false 在前。
4. `0x584ed0` 跳过 category15（FX tags）。`void/category21/source4/count0` 是 DB 默认占位记录，不是有效网络类别；采集器明确识别这一完整组合，其他解析错误仍失败。
5. `0x584d80` 对同一 category 的**此前资源**精确比较字符串字节并去重。当前资源内部重复项仍保留；直到整个资源过滤后，才更新已见集合。不同 category 的同名字符串独立保留。
6. 对剩余字符串按资源及原有字符串顺序连续 CRC32，无分隔符、无 NUL。结果与 `0x02df7d90` 对照；initialized=0 时该内存位置为 0，不能直接用作要发送的 CRC。

type44 布局为五个 32-bit 字段：name pointer、category、source、count、strings pointer。资源来自 hash buckets `0x01bc9f20[80000]` 和 entries `0x01fc4e28[100000]`（stride28，header+8、next+20），不可按旧 entry flags 过滤。类别布局 `0xc57118` 共21项，每项 (base, capacity, reserved)，总索引空间7058。

## 实测证据

原始文件均在忽略目录 `research-output/gamehub/`，不提交游戏资源。

| 采集 | 状态 | 计算/目标 CRC | 含义 |
| --- | --- | --- | --- |
| `client-1791341394600/net-constants-1791341795142.json` | 大厅0，initialized1 | 均为 `0x396299aa` | 公共 seed、排序、去重链独立复算通过；2672条贡献字符串 |
| 同实验 `net-constants-1791341887328.json` | mp_afghan加载7，initialized0 | 计算 `0x1f3a0a82`，内存0 | 55条资源记录，无读取错误；52条参与遍历，3205条贡献字符串 |
| `client-1791342759140/net-constants-1791342878732.json` | 9，initialized1 | 均为 `0x1f3a0a82` | Rust下发计算值后，客户端实际初始化结果一致；gamestateupdate1～6已通过 |

第二、三行的地图/模式来自本仓库受控训练赛单（`mp_afghan,tdm,1`，script `war`）及界面/分配实验。资源导出只有 `ncs_*_level` 名称，**不能凭这个 JSON 自动识别任意地图**；目前准备工具仅服务于该受控路径，不支持把其他地图采集自动标为可用状态。

## 复现

```sh
python3 tools/client_state.py --build --prefix-name wine10-probe-build-20261007
python3 tools/client_state.py --experiment research-output/gamehub/实际实验/experiment.json --net-constants
python3 tools/prepare_game_state.py research-output/gamehub/实际实验/net-constants-实际时间.json
```

使用已恢复的独立编译 prefix；不能在运行客户端的 prefix 内编译后执行 wineserver 清理。采集支持运行中的客户端，不调用游戏函数。

生成显式启用的训练配置时，对**确认是该训练地图的 state7 采集**加 `--output research-output/training-game-state.json`。输出包含原始采集 SHA-256、版本、地图/模式、计算 CRC 和计数，不包含常量资源正文。对于 initialized1 的采集，工具必须验证计算值等于目标观察值；对客户端版本不符、公共 seed 不匹配、记录数量错误、损坏 base64 和资源错误均拒绝继续。

测试使用合成资源，覆盖跨资源与资源内去重差异、跨类别同名、语言排序、忽略FX/占位符、seed和已初始化CRC不匹配。实际游戏资源仅用于本地验证。
