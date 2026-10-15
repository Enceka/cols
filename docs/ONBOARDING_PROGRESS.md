# 大厅初始化与只读客户端诊断

公共档案上传／服务重启／重登读取、私有档案、库存和大厅引导均已有本版原生验证。`onboard_complete` 为已核对的本地引导完成位，不等价于剧情或训练通关。独立新玩家的模板、等级和物品选项见[本地新玩家](LOCAL_PLAYER.md)，结构与偏移见[档案定义](PROFILE_INVENTORY_PROGRESS.md)。

## 初始化服务

| 任务 | 当前边界 |
| --- | --- |
| 8/1、8/3 | 当前身份公共档案读取／保存，校验 CRC、投影和归属 |
| 8/2 | 有效私有档案读取；经验启用时读取已提交投影 |
| 80/230 | 本地余额、库存及权益结构，库存 XP 使用账号投影 |
| 93/1 | CP 余额；币种和购买规则见[购买](PURCHASE_PROGRESS.md) |
| 80/173 | 券／邮件附件库存，支持已核对的空记录形状 |
| 80/70、80/107 | 本地空目录初始化；客户端内置商品预览不表示完整商城实现 |

未知任务、未支持筛选／分页及不合法参数明确失败，不以空成功掩盖缺项。非空附件、可选目录及完整商城业务仍有待恢复。

## 只读探针

```sh
python3 tools/client_state.py --build --prefix-name PROBE_PREFIX
python3 tools/client_state.py --experiment EXPERIMENT_JSON
python3 tools/client_state.py --assets --experiment EXPERIMENT_JSON
python3 tools/client_state.py --profiles --experiment EXPERIMENT_JSON
```

构建和运行变量见[运行手册](FOUNDATION.md)。探针先验证研究副本标记、固定 EXE 摘要、实验 PID、源码及二进制摘要，再对当前 prefix 的唯一目标进程读取固定范围。它不附加调试器、不调用游戏函数、不写内存、不读取会话密钥，也没有任意地址转储接口。

资源采集须等待对应 UI／地图完成加载；对象存在与资源 loaded 是不同状态，读取错误和缺失分别保留。当前快照、预测值和缓存可能不是同一采样瞬间，不能以非原子采样推断精确单帧因果。

客户端运行、独占锁、文件访问阻塞及临时配置／证书恢复见[启动与隔离](CLIENT_STARTUP_RECOVERY.md)。没有窗口或采样超时属于未取得原生验收，不能据此判断协议通过或玩法失败。
