# 公共配置、玩家数据与新玩家引导

更新：2026-10-06。当前接续入口。目标主程序 SHA-256 仍为 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。传输、认证及基础存储的早期证据见 [SERVICE_INITIALIZATION.md](SERVICE_INITIALIZATION.md)。

本轮已经解除必需公共配置和缺档案错误阻塞。真实 GameHub 客户端进入新玩家引导界面，能发出 ClientLogin，并将 `the.json` 上传后在重登时读取。截图中的界面仍出现“服务器繁忙，请稍候重试”；背包物品、装备保存、完整大厅及对局均未通过。

## 实验与验收边界

实验目录均在 `research-output/gamehub/`，使用原版主程序、独立运行副本和 GameHub 引擎 10：

| 目录 | 结果 |
| --- | --- |
| client-1791267783377 | 两个新生成公共文件各下载一次、不再重试；退出原因由公共文件失败变为 WIN_PROFILE_DOWNLOAD_FAILURE；首次出现 10/20 |
| client-1791268076178 | 增加 10/20、8/1 与 8/2 缺档案处理；首次出现 80/230；WineDbg 读取到配置版本=1、playlist 版本=1、服务器时间=1791268093；调试器附加引入 WOW64 异常，不能用本次评价稳定性 |
| client-1791268146961 | 无调试器对照，90 秒由工具主动结束；出现 ClientLogin、the.json 上传、TCP/UDP 心跳，无前述下载致命错误 |
| client-1791268311367 | 重新启动 Rust 和客户端，10/12 从磁盘读回 the.json；75 秒由工具结束，截图仍停留“连接中” |
| client-1791268514157 | 增加 80/230 单条空初始账号数据后进入新玩家引导；截图有“服务器繁忙”提示；新增 80/173，以及两次 8/3 公共档案上传；90 秒由工具结束 |
| client-1791268930437 | 接入公共档案持久化后的 75 秒实验；没有再次触发 8/3，不能宣称公共档案的真实上传—重登闭环通过 |

`the.json` 的真实上传为 36 字节，含结尾 NUL；持久化字节与原上传一致，SHA-256 为 `9f448e43006229c23a6f84f5905012a64ad3b3d84959ae4dd026948bee5d87b0`。重登回复 handler 为 `user_file`。这是社交列表文件恢复，不代表玩家装备档案恢复。

`client-1791268514157/game-window.png` 保留新玩家引导及忙碌提示；`client-1791268311367/game-window.png` 是之前“连接中”的对照。图片、票据、真实档案和会话日志都在忽略目录，未作为源码提交。

完整检查通过 39 项 Rust、5 项 Python 测试及 fmt/Clippy。两条真实 8/3 报文另由 `inspect-public-profile` 验证通过，输出记录在 `research-output/public-profile-validation.json`。公开 fixture 使用独立合成数据，未包含这些真实档案。

## configs_s.info：已从目标独立恢复并动态确认

完成回调 VA `0x00592190` 传入文件长度、数据指针和长度变量地址，调用 `0x0059fb60`。不要把反编译 C 中丢失的寄存器参数误读为指针参与 IV：本路径 IV 的前四字节仍是长度。

密钥恢复入口 VA `0x0059fa40`：

- 内嵌 RSA DER：VA `0x00c59fa0`，文件偏移 `0x00859fa0`，长度 `0x4a7`。
- 包装密钥块：VA `0x00c56ff8`，文件偏移 `0x00856ff8`，长度 `0x100`。
- 哈希名称为 SHA256；RSA-OAEP-SHA256 解包得到 24 字节资源密钥，独立结果与 AlphaBuild 的资源密钥一致。没有复制或提交整个 RSA 私钥。

目标算法是 AES-192-CTR：`IV = LE32(file_length) || 01×12`，128 位 counter 按大端递增，首块使用 AES(IV)。VA `0x00a852c0`、`0x00a82860`、`0x00a86740` 的初始化、重设 IV、递增指令已核对。wire 没有附带 IV、额外 padding 或 NUL。

`crates/protocol/src/publisher.rs` 实现该算法；`fixtures/protocol/publisher3333.json` 使用 PyCryptodome 独立生成，覆盖不同长度与多块输入。实际配置明文是 `configs/publisher/configs_s.ini`，只设置 `[version] g_dwConfigVersion=1`，并注明本地研究用途。它不是恢复出的官方全量配置。

目标 INI parser 为 `0x0058fde0`；`0x006cb4d0` 在 `0x006cb515..0x006cb534` 读取上述 section/key 并写到 `0x078a06bc`。动态该值为 1，证明不只是成功下载密文。

## release playlist：目标语法下的最小测试内容

完成回调 `0x005921d0` 尝试解压，然后允许原文本交给 `0x006111d0`。目标 parser 已核对 `version`、`gametype`、`script`、`teambased`、`catinfo`、`playlist`、`name`、`description`、`category`、`minparty`、`maxparty` 和地图/模式/权重记录。

`configs/publisher/mp_playlists_shipRetail_release.info` 提供版本 1、一个分类、一个 tdm/war 模式及 `mp_afghan,tdm,1`。本地资源和之前诊断中存在该地图名，**尚未验收进图或匹配可用性**。没有搬用旧 cb2 完整列表或配置武器/奖励。

动态 VA `0x031fe614` 为 1，证明客户端已解析版本。该最小列表仅为初始化测试数据，不表示支持了实时对局。

## 10/20 公共文件列表

构造器 `0x00a5b660`，调用者 `0x006f49e0`：context String、UInt32=0、UInt16=32、UInt16=0、终止 0。当前仅接受此空 namespace、无筛选/分页形状，其他参数明确失败。

结果对象数组起点 `0x0c96e308`，步长 `0xe8`，构造器 `0x00a5b130`，vtable `0x00c07e8c`，反序列化器 `0x00a5b230`。每项：

```text
UInt32 fileSize
UInt64 fileID
UInt32 createTime
UInt32 modifiedTime
Bool visibility
UInt64 owner
String filename       // capacity 128
String extraMetadata  // capacity 50；语义尚未确定
```

这里既不同于 AlphaBuild 的旧记录（增加最后一个 String），也不同于目标版 10/10 上传回执（该回执没有 fileID，带 accountType）。服务端列出实际存在的 `.info`/`.ff` 文件，稳定排序，用磁盘 mtime 表示更新时间；本地 fileID 由文件名 SHA-256 的前 8 字节生成。未知 metadata 字符串暂置空，实际初始化已接受，但后续缓存/热更新含义未验证。最多列出 32 项，未实现分页。

## 8/1、8/2、8/3 档案通道

`0x00a5a890` 构造 8/1（当前自己 UID 的 tagged UInt64），`0x00a5a9d0` 构造无参数 8/2。游戏回调 `0x006d83e0` 明确单独处理错误码 **800**，调用默认档案初始化；通用失败 4 会不断重试并最终报 WIN_PROFILE_DOWNLOAD_FAILURE。

目前 public 读取仅接受当前 AUTH UID；有已保存的 public.bin 就读回，无则返回 800。private 档案和 private 写入仍未实现，8/2 返回 800。跨 UID 请求明确拒绝，不把请求 UID 当作会话身份。

8/3 为当前用户公共档案写入：SDK 构造器 `0x00a5a420`，游戏序列化器 `0x006d7500`，反序列化器 `0x006d7740`。已捕获两条 1758/1832 字节请求，参数为：

```text
UInt32 compression    // 本次=1；也支持已核对的未压缩=0
UInt32 format         // 本次=1；非 1 拒绝
UInt32 CRC32
Blob profileSnapshot  // 压缩时为 zlib，解压后必须是 32768 字节
125 × tagged Int32    // 本样本当前 public schema 的索引字段投影
0
```

快照前四字节与 wire CRC 相同，CRC 覆盖后续 32764 字节。实际两条报文的校验分别为 `0x49350930`、`0x53ef74dd`，都与独立 Python zlib.crc32 计算一致。目标序列化使用 `mp/playerdata.def`；private 路径使用 `mp/playersecuredata.def`。没有使用旧版 220/11 档案种子。

读取结果先有 tagged UInt64 UID（`0x00a5a370`），然后是上述参数但不含请求终止 0。写入任务不绑定结果数组，所以保存成功后发送零记录状态回执。

实现位于 `crates/protocol/src/profile.rs`、`crates/server/src/services.rs`。解压限制固定为 32 KiB，拒绝 CRC 错误、压缩流尾随数据、膨胀和未知投影形状。只在完整校验后原子替换 `profiles/<title>/<UID>/public.bin`，保留客户端的精确序列化字节。测试覆盖重启读回、身份隔离及坏 CRC 不覆盖已存数据。完整游戏语义和索引字段还未解析；其他字段投影暂不接受。

## 80/230 初始玩家数据

请求构造器 `0x00a472b0`，调用者 `0x004a5080`/`0x004a5120`；参数仅为空 context String。结果绑定 `0x07bdc6f8`，对象大小约 `0x8010`，构造器 `0x00a4a5b0`、解码器 `0x00a4a6b0`。

当前返回 **一条完整结构的空初始账号记录**：UInt64 UID + String accountType、UInt32 余额数 0、UInt32 物品数 0、UInt32 权益对数 0。权益解码器 `0x00a4bd80` 从父对象继承用户身份；其计数虽然在内存中截为 UInt16，wire 仍是 UInt32。UID=-1 的 SDK sentinel 情况需要额外编码账户身份，已单独处理。

这不是将未知任务统一置为空成功；上述结构经目标解码器恢复，并实际推动界面从“连接中”到新玩家引导、产生 80/173 及 8/3。当前没有授予起始武器/角色、余额或权益；目录 80/70 与新出现的 80/173 仍明确失败。因此不能宣称背包恢复。

## 运行与继续工作

```sh
sh tools/check.sh
sh tools/cargo.sh build --workspace --locked

# 仅首次生成；文件已存在时拒绝覆盖
target/debug/codol-server prepare-publisher \
  --output-dir client-work/server-data/publisher

python3 tools/gamehub_run.py client --seconds 90 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun \
  --local-services --trust-lab-cert
```

`inspect-public-profile --input <packet-NNN-message.bin>` 只验证解密后的 8/3 请求，不导入数据，不输出原始档案。静态导出包括 publisher-parse.c、storage-list.c、profile-tasks.c、playerdata-decode.c、profile-codec.c，均在 research-output/ghidra 下。Ghidra 仍会因调用约定错误删去有效分支；必须核对原始指令。

WineDbg 能读游戏内存，但本环境 `winedbg <wpid>` 的附加线程触发地址 `0xffc0fbf0` 的异常，随后真实诊断为 ClientException。暂不要用该附加方式做稳定性实验；无调试器对照可以继续运行。需要额外内存观察时优先考虑不附加的 ReadProcessMemory 工具或 Parallels 对照。

后续顺序：

1. 对 8/3 持久化进行真实 UI 上传和重登回读验收；本轮最终运行没有重新触发上传，不要把历史抓包离线验证写成现场成功。
2. 定位 80/173（实测参数为空 context、UInt16=700）及 93/1，结合新玩家引导的“服务器繁忙”提示判断必需任务；继续补齐 80/70 目录。
3. 从目标资源恢复起始角色/武器和玩家档案字段，提供可操作背包与装备保存。公开/私有档案、玩家数据、物品目录是不同通道，不能混为一次“背包响应”。
4. 完整大厅与背包验收通过后，再推进双客户端对局。研究连接仍限 120 秒，稳定性验证要明确区分主动截止与错误退出。

用户已要求建立 Git 提交；之后每个通过验证的阶段及时提交源码、合成 fixture 与文档，保留可接续的提交历史。真实客户端、账户数据、截图、会话密钥、TLS 私钥及实验捕获继续留在忽略目录。
