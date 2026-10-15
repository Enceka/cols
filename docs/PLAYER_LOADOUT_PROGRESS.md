# 玩家背包、武器注册与物品所有权

Rust按认证账号保存的当前角色背包装配武器，训练诊断与正常团队准备分开维护。正常入口见[本地游玩](LOCAL_PLAY.md)，武器行为范围见[原生武器](WEAPON_NATIVE_PROGRESS.md)。

## 账号与资源

账号按admission的title／owner读取public.bin，校验CRC和125项投影；所有权由基础库存与真实购买合并。本版只读arsenal含188个基础定义、默认槽和兼容附件，compact registry的ID别名须与独立marketplace表交叉核对。

首次装配按当前角色×5加角色内背包号读取真实 `weaponClasses` 偏移；默认该角色首包，显式选择保持1..5位置。500ms档案刷新保留角色内背包号，不退回固定全局记录。只因组合没变不能忽略技能、投掷物和连杀选择的变化。

默认六部件按kind被玩家选择覆盖，纯Rust合成本版12字节属性记录，生成容量、射速、换弹、ADS、切枪和移动参数。默认／单配件真实资源解析及部分组合有独立对照，不能推断全部特殊武器通过。

## 注册与应用

武器名称后发布 `CS6800=最高武器编号+1`，资源可靠ACK后才换装。同账号重连保持既有注册编号，原生替代模式紧随父武器。主副枪分别保存弹药和状态，重复请求不重填弹药。

原生菜单支持 `custom1..custom5`。团队准备期及未交战开局宽限即时应用已确认背包，其它活动生命在下次出生应用；共享多人中途切换的等同范围仍需验收，见[比赛流程](MATCH_FLOW_AND_SPAWNS.md)与[多人](MULTICLIENT_ACCEPTANCE.md)。

## 全物品

`tools/unlock_all_items.py` 按本版6772项物品表授予所有权，默认只展示计划，`--apply`在离线资料上执行并保留备份，重复操作幂等。已有购买、公私档案和钱包保留，不将未知unlockFlags置极值。UI等级、武器经验和剧情门槛分别核对。

库存上限8192，S2C任务／加密体预算512KiB，继续核对协商max_receive及C2S边界。真实6772项库存及刷新已通过服务和TCP传输验证。

```sh
python3 tools/prepare_player_loadout.py \
  --state GAME_STATE_JSON --assets ASSETS_JSON --arsenal ARSENAL_JSON \
  --collision COLLISION_JSON --output NEW_GAME_STATE_JSON
```

生成器拒绝覆盖。正常运行读取准备文件，不读取客户端内存；探针仅用于资源采集。角色0／1／7／11与多个背包、非连续偏移、所有权、注册和ACK均有回归；更多角色首次武器及完整特殊枪械视觉见[复核清单](REVIEW_PENDING.md)。
