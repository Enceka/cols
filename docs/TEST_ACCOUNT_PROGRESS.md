# 测试账号、等级与钱包

本地账号按 title／UID 隔离。诊断使用独立服务资料，不重置日常账号；全物品、人物等级、武器经验和界面解锁分别处理。新玩家入口见[本地新玩家](LOCAL_PLAYER.md)。

`boost-test-account` 是离线维护命令：备份已有测试资料，设置本版人物等级 70 和显式 CP／GP。普通人物等级为 1～70，rankTable 版本为 4；人物经验保存为级内经验，不能直接写入累计门槛。`uf_prestige_rank` 的普通等级下限与玩家等级一致，已解决原生建房等级校验；它不是武器等级。

```sh
target/release/codol-server boost-test-account \
  --service-data-dir SERVICE_DATA_DIR --title-id 1002 --user-id TEST_UID \
  --cp 100000 --gp 1000000
```

工具只用于已确认归属的离线测试资料，不能与运行中的账号写入并发使用。已有经验事务的导入种子拒绝覆盖，需显式迁移。购买与余额持久化见[购买](PURCHASE_PROGRESS.md)，自然升级和结算回执见[等级](PROGRESSION_PREPARATION.md)。

全物品仅授予本版 6772 项物品所有权，不修改未知解锁位，不代替剧情进度或全部装备效果。建房、原生地图和对局范围见[房间](ROOM_PROGRESS.md)与[地图覆盖](MAP_COVERAGE.md)。
