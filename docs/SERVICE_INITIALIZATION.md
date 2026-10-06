# 存储、时间及初始化阻塞点

更新：2026-10-06。当前接续入口。目标主程序 SHA-256：`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。此前 AUTH、协议 210 和 UDP v3 发现的证据见 [AUTH_LOBBY_PROGRESS.md](AUTH_LOBBY_PROGRESS.md)。

后续已解除公共配置阻塞并进入新玩家引导；最新接续入口为 [PUBLISHER_FILES.md](PUBLISHER_FILES.md)。下文保留本阶段当时的发现与验证边界。

本轮增加 Rust 任务分发、服务器时间、用户文件读写及公共文件读取。用户文件按已认证 title/UID 隔离，可在进程重启后读取；这仍是透明文件存储，不是背包、装备或玩家档案的业务实现。真实客户端已经走到公共配置下载阶段，并明确报出 `EXE_DOWNLOAD_PUBLISHER_FILE_FAILURE`。第一阶段仍未完成。

## 真实实验与验证边界

`research-output/gamehub/client-1791266444699` 使用 GameHub 引擎 10、本项目运行副本、原版主程序，以及同一个 Rust AUTH/大厅/STUN 进程。开启新的本地服务模式，记录 34 个请求及 34 条按序回执：

| 处理结果 | 数量 | 验证范围 |
| --- | ---: | --- |
| 71/4 注册 | 1 | 延续此前已验证的状态回执 |
| 12/6 时间 | 1 | 发出一条单记录 UInt32 秒数响应；尚未读取客户端全局时间作最终确认 |
| 用户文件缺失 | 2 | `the.json`、`FileBriefMessage` 返回 1000 |
| 公共文件缺失 | 14 | 初始化请求及必要文件重试均返回 1000 |
| 未实现任务 | 16 | 明确返回终止失败 4，维持普通任务 FIFO |

客户端在约 25 秒内退出 255，未触及新的 256 帧限制。最后的 71/7 诊断明文包含 `EXE_DOWNLOAD_PUBLISHER_FILE_FAILURE`，对应目标字符串 VA `0x00c6be3c`、报错调用 VA `0x0058ec6e`。这次实验没有用户文件上传，因此**真实客户端上传、重登读回、背包和完整大厅 UI 均未通过**。持久化重启读回目前由合成协议测试证明。

截至本轮 `sh tools/check.sh` 通过 32 项 Rust 测试、5 项 Python 测试及 fmt/Clippy。新增覆盖：有界 tagged 解析、截断/溢出、独立手写回执字节、用户文件重启读回、title/UID 隔离、路径及坏文件拒绝、未知任务后流水发送时间/文件请求的 FIFO 集成测试。一次原有观察器测试因 75ms 空闲窗口过窄失败，已改为 500ms 并加入失败事件输出；完整检查随后通过。

## 回执关联：不能跳过未知任务再回复后面的任务

目标普通消息 kind=1 由 VA `0x00a3c090` 处理：先取任务链表头，交给 `0x00a44f30`，再移除队首。transaction ID 从响应读入任务对象，并非请求携带、服务器回显的关联 ID。普通响应必须按请求顺序发送。另有异步响应处理路径 `0x00a3b9a0`，本轮没有实现。

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

**反编译器陷阱：** 初始 Ghidra C 将 `0x00a44f30` 的成功分支错误地消去，输出了始终失败的伪代码。这源于未经修正的底层调用约定/输出参数推断；实际指令 `0x00a44fa2..0x00a45061` 存在完整成功与错误路径。已保存 `research-output/ghidra/service-wire-disassembly.json`；涉及输出参数时应核对指令，不能只信 C。

原来的纯观察模式保留：仅回答 71/4，其余任务不回复。它适合抓取，不能用于选择性回答后续任务。新模式对每个可识别的 kind=0x86 请求生成成功或失败回执。

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

## 公共配置是当前明确阻塞点

目标公共文件完成回调 `0x0058eb50` 读取 `0x00cdd260 + fileIndex` 的必需标志。已核对前八个标志为 `01 01 00 00 00 00 00 00`。派发表 VA `0x005920f8` 的前两项对应：

- index 0：`configs_s.info`，派发分支 `0x0059201a`、完成回调 `0x00592190`。
- index 1：动态拼装的 playlist 名称，本样本请求 `mp_playlists_shipRetail_release.info`，完成回调 `0x005921d0`。

必需文件未成功取得时，`0x0058ec44` 递增重试次数；超过 3 或重试启动失败会报上面的致命错误。实际实验也只看到这两个文件重复下载。不能通过给所有文件返回空成功来认定恢复：文件内部解析还有各自要求。

参考 AlphaBuild 提供旧版 `configs_s.info` 的 AES-192-CTR 实现和旧 playlist，但目标版的密钥、数据格式、地图/模式引用尚未逐项验证。**不要直接把旧版密文、cb2 playlist 或 220/11 玩家档案当成目标版数据。** 下一步应从上述两个完成回调恢复解密及解析链，再生成符合本地测试范围的配置。

## 80/70：已知结构，语义待定

构造器 `0x00a462a0`，调用者 `0x004a6020`。参数为 context、两个 UInt32、Bool、UInt32 元素数组、UChar 元素数组、容量 32 的 String；本次值为：空 context、1、2000、true、两个空数组、`982808`。它使用分页形状，结果数组起点 `0x07f34208`，对象步长 `0x7d0`，构造器 `0x00a49d00`、解码器 `0x00a49a40`，后者先调用 `0x00a497f0` 再读一个 UChar。

这些证据更像商城目录/SKU 查询，尚不能定名为 getPlayerData 或 getInventory。当前返回明确失败，未发送旧版 80/30 数据。相关 C 导出在 `services-init.c`、`record-types.c`、`file-records.c`，仍需指令与调用者消费逻辑交叉核对。

## 实现入口、运行及接续

- `crates/protocol/src/task.rs`：有界 tagged reader、存储请求、普通回执编码。
- `crates/server/src/services.rs`：时间、文件服务、title/UID 绑定、原子文件替换。
- `crates/server/src/lobby_server.rs`：按序调用处理器，写入 `packet-NNN-reply.json`，再加密发送。
- `crates/server/tests/lobby_transport.rs`：未知任务失败后仍能按序取得时间及缺文件回执。

```sh
sh tools/check.sh
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --seconds 90 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun \
  --local-services --trust-lab-cert
```

`--local-services` 是新开关；省略可复现此前观察模式。直接启动 Rust 时设置 `serve-auth --service-data-dir client-work/server-data`，其余 AUTH/大厅/STUN 参数见前文档。

数据目录：`publisher/<filename>` 存放操作者提供的真实公共文件；`users/<title>/<UID十进制>/<filename>.json` 保存 schema、时间、visibility 和 Base64 blob。UID 始终为 u64；磁盘目录用十进制字符串。初始 publisher 目录为空。文件大小目前限制 60 KiB，每用户最多 128 项；不支持分片下载、大文件、SQLite、非空 namespace 或完整 playerData。

本地服务模式最多捕获 256 帧、1 MiB，单连接 120 秒、认证后空闲 60 秒；纯观察模式仍为 32 帧。它们都是研究限制，不能据此宣称长期大厅稳定性。持久化使用同步临时文件后原子 rename；测试覆盖进程重启，不宣称已完成断电恢复测试。

下一轮优先：恢复 configs_s 与 release playlist 的解密/解析；再次运行真实客户端；确认时间全局值及实际文件上传/重登；继续确定 80/70 目录与真正的 playerData/inventory 请求，随后实现物品及装备状态。
