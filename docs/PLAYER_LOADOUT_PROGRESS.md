# 玩家背包、武器注册与全物品授权

2026-10-07。当前配置入口为 `research-output/training-dynamic-gameplay-state.json`。Rust运行时根据玩家保存的背包读取物品、兼容部件和属性，不再依赖两套实测配装预设。旧 `training-player-backpack-state.json` 是本轮早期诊断文件，不应用于后续运行。

## 背包进入对局

- 账号来源：admission绑定的title/owner，读取最新public.bin并校验CRC及125个投影；库存由inventory和真实购买记录合并。
- 资源来源：本版只读arsenal捕获，含188个基础武器定义、默认槽位与兼容附件。compact item registry存在ID别名，必须与独立marketplace表的ID/名称交叉核对。
- 装配：按部件kind替换对应默认部件，保留其他部件；纯Rust合成12字节属性记录，按本版排序和运算生成容量、射速、换弹、ADS、切枪与移动参数。
- 更新：每500ms检查保存内容；组合没变不重复装备。新组合先发送可靠武器CS，等待ACK后更换玩家武器；已注册的组合名复用原编号。仅修改配件不会给未变副枪重新分配编号。
- 当前训练选定背包1，可在准备时用 `--backpack` 指定其他编号。对局内菜单切换背包未接入。支持普通自动和半自动枪的状态机；burst、特殊射击和近战仍有明确拒绝条件，不能把188个资源可解析等同于全部玩法已实现。

动态资源文件：`gamehub/client-1791379955087/arsenal-1791380211328.json`。默认和单附件共2712次真实资源解析通过；消音器+FMJ结果与客户端实测一致。当前用户又更换到红点+握把，随后实际进入AK117配装，说明运行路径已不再固定M4配件。

## 切枪崩溃及修复

客户端上报异常栈的关键链为 `aa61a4 → 449cdd → 44da4c`：HUD读取未初始化的武器文字指针。目标0x558540写入CG weapon presentation记录，进图0x4435fa和动态0x5597d0均以CS6800为注册数量。

服务器现在在武器名称配置之后发送 **CS6800=最高武器编号+1**；动态新增时同样先名称、后数量，并等待可靠ACK。这保证M9等备用武器在切枪前已有显示名称与客户端表现资源。用户已实机确认“可以切枪了”。不要通过隐藏HUD回避此错误。

纯Rust Player保存主/副枪各自弹药和状态，执行普通/空仓收枪→举枪；半自动按下单发，生成开火、换弹、补弹、收枪、举枪、脚步等目标版事件。弹丸命中、伤害和完整特殊武器尚未实现。

## 全物品

用户明确要求全物品解锁并暂缓商城价格工作。`tools/unlock_all_items.py` 基于本版完整物品表授予 **6772项**，原8项扩展新增6764项；保留9条购买、公私档案和钱包原字节，未修改价格或执行购买。

已于本轮停服窗口执行 `--apply`；完整账号备份与manifest位于本地账号目录的 `backup-unlock-all-331b75ed591b414b`，原始账号资料不入Git。工具默认只展示计划，重复执行幂等。未将未知private unlockFlags整体置极值；如果UI仍出现实际门槛，需按本版具体字段和minimum验证处理。

为使全部物品真正送达，将本地库存预算提高到8192项，S2C任务/加密体预算提高到512KiB，保持已协商max_receive检查及C2S原有边界。目标SDK库存为动态数组，UI复制上限20000；真实ClientHello协商512KiB。6772条库存和刷新均已通过服务测试与真实TCP加密传输测试。

## 重现

```sh
python3 tools/prepare_player_loadout.py \
  --state research-output/training-complete-m4-state.json \
  --assets research-output/gamehub/client-1791285213734/client-assets-1791285369232.json \
  --arsenal research-output/gamehub/client-1791379955087/arsenal-1791380211328.json \
  --collision research-output/gamehub/client-1791379955087/collision-1791380213516.json \
  --output research-output/training-dynamic-gameplay-state.json
```

准备器拒绝覆盖；重复生成时换新输出名。正常服务无需读取实际客户端内存；只读探针仅在离线恢复本版资源时使用。仍使用纯Rust运行时，无FFI或注入。

移动与音频的验证状态见 [GAMEPLAY_INPUT_PROGRESS.md](GAMEPLAY_INPUT_PROGRESS.md)。

## 2026-10-11 当前角色的初始背包

实际进入对局的准备流程曾直接读取地图配置里的全局`class_index`，通常为0。虽然局内`mr ... customN`已会用当前角色索引×5选择背包，首次开局仍可能给其它角色使用角色0的枪。

现于认证账号档案读回之后、武器装配与注册之前，先用本版`currentCharacterIndex`和配置里的角色内背包号选择对应的`weaponClasses`记录。默认仍为该角色第一个背包；显式配置其它背包保留其1..5位置，不把角色0的全局记录号当所有角色通用。连接同时保存该角色内背包号，后续500ms档案刷新不会再次退回模板中的固定全局记录。

回归通过实际公共档案信封/CRC和125项投影验证路径，覆盖角色0/1/7/11及背包1/4/5，分别核对主枪、六个附件、副枪和非连续偏移。只在内存构造测试档案，不改现有账号。用户实际切换角色后的画面与首次背包界面列入 `REVIEW_PENDING.md`，不阻塞后续功能。
