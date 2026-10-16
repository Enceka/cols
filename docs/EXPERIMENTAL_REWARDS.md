# 实验性连杀资源

捕食者（5）和攻击直升机（8）在本图资源完整时开放调用。地图目录中的23份战术、9份冲锋准备可绑定本图实际NCS中的奖励武器、模型、载具、特效、声音和文本槽；共享定义、边界和时序沿用已经采集的公共资源。

这是实验性开放，不保证每张地图都能正常发动，也不保证完整命中、碰撞、航线、声音、画面或运行稳定性。室内和黑房地图尤其可能异常。缺少完整原生空域时，可以使用依据本图出生点生成的兼容航线；它不是原生捕获。沼泽和庄园的资源可显式设置 `experimental_air_routes=true`，允许尝试不能通过保守净空预检的航线；运行时碰撞处理仍保留。该标志缺省为false，不改变其它资源的预检条件。

开放开关和资源绑定不等于完整原生验收，也不会开放仍未完成的7／10／12／13／18／20／23等控制器。逐项状态见[覆盖表](KILLSTREAK_COVERAGE.md)。

## 发奖和背包

3／5／7配置在第3、5、7杀逐项发奖，第二轮为10、12、14杀，第三轮为17、19、21杀。只有本次实际计数恰好到达该项门槛才发奖，不在较晚节点集中补发被跳过的低阶奖励；死亡重置本条命的计数。使用奖励不重置进度，同种未使用奖励最多保留一份，技能直接及连锁击杀不推进奖励计数。

新玩家模板会给整组空白的60个背包连杀槽配置UAV／捕食者／攻击直升机，并赋予初始所有权。完整或部分自定义选择保持原样，已有模板和账号的修复入口见[本地玩家](LOCAL_PLAYER.md)。每个背包独立保存三项奖励；界面上的可操作按键来自实际库存，不是配置预览。

## 准备目录中的全部地图

准备工具需要原地图目录、本图初始化NCS及可选地图实体捕获、完整公共奖励和核弹资源。输入保持不变，输出须为新目录；失败不发布半成品。输入索引支持以下结构，路径相对于索引文件：

```json
{
  "schema_version": 1,
  "entries": [{
    "map": "mp_nuked",
    "mode": "war",
    "ncs": "mp_nuked/constants.json",
    "entities": "mp_nuked/entities.json"
  }]
}
```

NCS必须为本图已初始化、匹配客户端摘要且CRC与原准备一致的只读采集。实体记录若提供也必须属于本图；缺少实体时才使用明确标注的兼容空域。核弹共享文件不得携带诊断起始进度、静止Bot或其它验收开关。

```sh
python3 tools/prepare_catalog_rewards.py \
  --catalog /path/to/original/catalog.json \
  --capture-index /path/to/capture-index.json \
  --shared-resources /path/to/verified/shared-rewards.json \
  --shared-nuke /path/to/verified/shared-nuke.json \
  --compatibility-airspace mp_bog_sh,mp_estate_tropical,za_coliseum \
  --unchecked-air-routes mp_bog_sh,mp_estate_tropical \
  --output /path/to/new-reward-preparation

# 转为可独立迁移的运行目录，并调用真实加载器校验全部依赖。
target/release/codol-server prepare-resources \
  --source /path/to/new-reward-preparation/catalog.json \
  --output /path/to/new-runtime \
  --report /path/to/new-runtime-report.json
```

仅设置人物等级或授予全部物品不会生成缺失的地图资源。运行服务端和启动器时不需要Python，准备转换才需要随源码提供的工具。

## 验证范围

合成回归逐杀检查三轮3／5／7发奖、HUD槽位、消费、跳过门槛不补发，以及实验性航线许可的边界。Python回归检查真实NCS槽位重绑、错误CRC／重复资源拒绝、新目录原子发布、空背包填充和原配装保留。实际32份准备已逐图通过资源加载及两种控制器调用计划检查；这不表示32份都经过完整原生对局验收。

阿富汗原生隔离样本已显示4／5／6可操作按键，实际触发捕食者和直升机的服务端调用。该效果样本使用诊断起始进度，不作为自然零起始逐项发奖的证据；连续三轮的逐杀时机由独立回归检查。完整命中、长期运行及共享多人效果继续按专题验收。
