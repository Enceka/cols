# 测试账号等级、钱包与功能限制

更新：2026-10-06。当前接续入口。用户反馈：等级显示提高后，**创建房间仍有等级限制，购买仍失败**。这两项不能用截图上的数字代替验收。

## 已实现与实测

- 新增离线 `boost-test-account`：验证已有 private.bin 的 CRC、投影和 UID，备份原文件，再设置 rank=70、rankVersion=4、experience=0。保留其他字段、public.bin 和 inventory.json。
- 经验单位来自目标指令：0x590720 返回 rank-1；0x590550 将 experience 加到 rankTable 第 2 列门槛。因此 experience 是级内经验，不应写累计经验 3582550。GetPlayerLevel 的绑定 0x6a4fe0 → vtable 0xc7a86c → 0x5a97e0 同样调用 0x590720。
- 按 title/UID 保存 wallet.json，记录目标样本哈希、schema_version=1 和 cp/gp。当前测试值 CP=100000、GP=1000000。没有文件的账号维持零余额；拒绝错账号、错版本、坏 JSON。
- 余额嵌套解码器 0xa64ad0：继承账号，再读取 UChar 币种、UInt32 数额。关键修正：**0x4a5680 只复制 80/230 的第一条余额**到实际钱包，因此 80/230 只返回 currency=2；currency=1 由 93/1 单独返回。
- UI GetCP：绑定 0x6b1080 → vtable 0xc7b4c4 → 0x6af9e0，扫描币种 1；另一 getter 0x6af970 扫描币种 2。0x5e28ce 将币种 1 写入 UI codPoint，0x5e2941 将币种 2 写入 moneyPoint。应按目标 getter 解释，不能单凭图标猜币种。
- 实验 client-1791279640378：等级实际显示 70；最初把两种余额都放进 80/230，SDK 解析成功，但实际钱包漏了币种 2。此实验不能算双余额生效。
- 修正后的 client-1791280038105：client-state-1791280088002.json 读到 marketplace_currency=2、marketplace_balance=1000000，external_currency=1、external_balance=100000，库存仍为 8 件。ui-1791280164849.png 显示等级 70 和两种余额。

## 尚未解决

- 用户明确指出“创建房间有等级限制”。需观察精确提示与触发条件；不能通过批量清除所有锁定标志来掩盖原因。
- 真实操作已产生 11/13 创建房间和 21/5 搜索请求；当前服务端仍返回未实现错误，点击后可快速重复请求并耗尽 256 帧实验预算。
- 购买分别产生 93/131 与 80/154，当前均未实现。客户端能展示内置价格，不代表购买服务已恢复。必须补齐校验、扣款、发货、失败处理和重登持久化，才能称购买可用。
- 93/131 构造器 0xa4c0b0，序列化委托 0xa64ca0；80/154 构造器 0xa467e0，委托 0xa45b40。最新静态结果在 research-output/ghidra/purchase-and-room.c。
- 当前仓库仍只发放已核实的 8 件库存，商品目录返回空。等级 70 不代表所有武器等级或装备权益已解锁。

## 使用与恢复

先停止本项目客户端与所有服务进程，再执行：

```sh
sh tools/cargo.sh build --workspace --locked
target/debug/codol-server boost-test-account \
  --service-data-dir client-work/server-data --title-id 1002 \
  --user-id ACCOUNT_UID --cp 100000 --gp 1000000
```

工具只处理已有私有档案。每次在账号目录创建 backup-test-account-*，内含原 private.bin、原 wallet.json（若存在）和 manifest.json。单文件替换是原子的，两文件并非事务；如中断，停止服务后按备份恢复 private.bin，并根据 wallet_existed 恢复或移除 wallet.json。不能拿旧 public.bin 覆盖客户端的新保存。

本机首次应用记录 research-output/test-account-boost.json，备份目录后缀 305b66514ccee6dd。完整档案、UID、截图和抓包继续留在忽略目录。

49 项 Rust、11 项 Python 测试覆盖字段保留、备份、重启读回、身份隔离、协议标签和余额读取；真实创建房间与购买仍需独立动态验收。
