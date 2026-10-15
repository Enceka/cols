# 任务分发、时间与文件存储

本地服务已提供时间、用户文件、公共配置、档案、库存和房间等初始化业务。必需公共配置可以生成并被客户端解析，大厅不再受配置下载阻塞；具体格式见[公共配置](PUBLISHER_FILES.md)，档案和库存见[档案定义](PROFILE_INVENTORY_PROGRESS.md)。

## 回执关联：不能跳过未知任务再回复后面的任务

目标普通消息 kind=1 由 VA `0x00a3c090` 处理：先取任务链表头，交给 `0x00a44f30`，再移除队首。transaction ID 从响应读入任务对象，并非请求携带、服务器回显的关联 ID。普通响应必须按请求顺序发送。另有异步响应处理路径 `0x00a3b9a0`，尚未实现。

`0x00a44f30` 的指令确认：

```text
tag10 UInt64 transactionID
tag8  UInt32 error
若 error == 0:
  tag3 UChar task
  tag8 UInt32 resultCount
  若 resultCount != 0:
    tag8 UInt32 totalResultCount
    result records
```

`error=200` 使任务保持 pending；其他非零值使其失败。目标自身在处理/反序列化失败时使用 4。本地服务模式用 4 表示不支持或拒绝的任务，不宣称它是官方“未实现”错误码。缺文件使用 1000；VA `0x006f47a0` 明确单独处理这个值。

**反编译器陷阱：** 未经核对的 Ghidra C 可将 `0x00a44f30` 的成功分支错误地消去，输出了始终失败的伪代码。这源于未经修正的底层调用约定/输出参数推断；实际指令 `0x00a44fa2..0x00a45061` 存在完整成功与错误路径。已保存 `research-output/ghidra/service-wire-disassembly.json`；涉及输出参数时应核对指令，不能只信 C。

纯观察模式：仅回答 71/4，其余任务不回复。它适合抓取，不能用于选择性回答后续任务。本地服务模式对每个可识别的 kind=0x86 请求生成成功或失败回执。

## 已恢复的目标版本格式

以下字段均带类型标签，标量小端；String 为 tag16 + NUL 结尾字节串，Blob 为 tag19 + tagged UInt32 长度 + 数据。请求最后有一个 NoType=0；它不属于 blob，blob 内部可以另有 NUL。

| 任务 | 构造器 VA | 实际参数 |
| --- | --- | --- |
| 10/12 读取用户文件 | `0x00a5b570` | context String、filename String、owner UInt64、accountType String、0 |
| 10/21 读取公共文件 | `0x00a5b7f0` | context String、filename String、0 |
| 10/10 上传用户文件 | `0x00a5b400` | context String、filename String、visibility Bool、Blob、owner UInt64、accountType String、UInt32、0 |
| 12/6 获取时间 | `0x00a4eca0` | 0；没有其他参数 |

当前观察到的 context/accountType 均为空，上传 owner=0、最后 UInt32=0。最后的 UInt32 在代码中命名 version，**业务含义尚未确认**；非零值明确拒绝，不擅自实现版本冲突语义。用户读请求 owner 必须等于 AUTH 绑定的 UID；上传 owner=0 表示使用当前会话身份，也接受相同 UID，不允许切换身份。

读取用户/公共文件的成功结果是一个 blob。目标对象构造器 `0x00a4e800` 的 vtable 在 `0x00c0783c`，反序列化函数 `0x00a4e8b0` 调用 `0x00a392b0`，后者读取 tag19、tag8 长度和字节。

上传成功绑定文件信息对象：调用者 `0x006f4860`，构造器 `0x00a4e960`，vtable `0x00c07848`，反序列化器 `0x00a4eab0`。目标字段顺序为：

```text
UInt32 fileSize
UInt32 createTime
UInt32 modifiedTime
Bool visibility
UInt64 owner
String accountType
String filename
```

时间与大小的名称根据字段用途及参考语义归纳；类型和顺序已核对指令。**新版这里没有 AlphaBuild 的 UInt64 fileID**。用户账户的组合解码器 `0x00a45950` 读取 UInt64 + 容量 10 的 String。

时间结果为一个 tagged UInt32 Unix 秒数：构造器 `0x00a5ca90`、vtable `0x00c07f10`、反序列化器 `0x00a5cb20`。客户端接收回调 `0x006fca30` 把结果写入 VA `0x0c9e34e0` 并重置计时器；动态读取这个地址可作为进一步验收。

## 持久化与边界

用户文件按已认证 title／UID 隔离，通过临时文件和原子 rename 保存。文件上传不等价于业务授权；档案、库存、购买和经验各自校验归属及结构。账号写锁和购买扣款／授予边界见[模块结构](SERVER_MODULARIZATION_PLAN.md)。

持续运行及采集预算见[本地游玩](LOCAL_PLAY.md)，不能将观察器的有限预算套用于正常对局。文件不存在、不支持任务和格式错误分别返回明确失败，不能统一空成功。
