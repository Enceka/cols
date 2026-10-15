# 公共配置与玩家初始化协议

必需公共配置已按本版解密和解析链恢复。`prepare-publisher` 将仓库 UTF-8 模板生成客户端使用的编码文件；不能直接将 UTF-8 源模板复制进服务数据目录。赛单和已准备地图由本版资源身份绑定，见[地图入口](NATIVE_MAP_ENTRY_RULES.md)。

## configs_s.info：已从目标独立恢复并动态确认

完成回调 VA `0x00592190` 传入文件长度、数据指针和长度变量地址，调用 `0x0059fb60`。不要把反编译 C 中丢失的寄存器参数误读为指针参与 IV：本路径 IV 的前四字节仍是长度。

密钥恢复入口 VA `0x0059fa40`：

- 内嵌 RSA DER：VA `0x00c59fa0`，文件偏移 `0x00859fa0`，长度 `0x4a7`。
- 包装密钥块：VA `0x00c56ff8`，文件偏移 `0x00856ff8`，长度 `0x100`。
- 哈希名称为 SHA256；RSA-OAEP-SHA256 解包得到 24 字节资源密钥，独立结果与 AlphaBuild 的资源密钥一致。没有复制或提交整个 RSA 私钥。

目标算法是 AES-192-CTR：`IV = LE32(file_length) || 01×12`，128 位 counter 按大端递增，首块使用 AES(IV)。VA `0x00a852c0`、`0x00a82860`、`0x00a86740` 的初始化、重设 IV、递增指令已核对。wire 没有附带 IV、额外 padding 或 NUL。

`crates/protocol/src/lobby/publisher.rs` 实现该算法；`fixtures/protocol/publisher3333.json` 使用 PyCryptodome 独立生成，覆盖不同长度与多块输入。实际配置明文是 `configs/publisher/configs_s.ini`，只设置 `[version] g_dwConfigVersion=1`，并注明本地研究用途。它不是恢复出的官方全量配置。

目标 INI parser 为 `0x0058fde0`；`0x006cb4d0` 在 `0x006cb515..0x006cb534` 读取上述 section/key 并写到 `0x078a06bc`。动态该值为 1，证明不只是成功下载密文。

## release playlist：目标语法下的最小测试内容

完成回调 `0x005921d0` 尝试解压，然后允许原文本交给 `0x006111d0`。目标 parser 已核对 `version`、`gametype`、`script`、`teambased`、`catinfo`、`playlist`、`name`、`description`、`category`、`minparty`、`maxparty` 和地图/模式/权重记录。

`configs/publisher/mp_playlists_shipRetail_release.info`是本地赛单模板，包含人机训练与团队入口，地图／人数按本版规则生成。实际已准备目录支持`war`／`cha`，特殊入口按原生标志和容量分开处理；见[房间](ROOM_PROGRESS.md)和[地图入口](NATIVE_MAP_ENTRY_RULES.md)。赛单被解析不代表对应资源或玩法可用。

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

目前 public 读取仅接受当前 AUTH UID；有已保存的 public.bin 就读回，无则返回 800。private 档案读取由有效种子或经验事务投影提供，缺失时返回 800；私有业务写入不通过任意上传完成。跨 UID 请求明确拒绝，不把请求 UID 当作会话身份。

8/3 为当前用户公共档案写入：SDK 构造器 `0x00a5a420`，游戏序列化器 `0x006d7500`，反序列化器 `0x006d7740`。已捕获两条 1758/1832 字节请求，参数为：

```text
UInt32 compression    // 样本=1；也支持已核对的未压缩=0
UInt32 format         // 样本=1；非 1 拒绝
UInt32 CRC32
Blob profileSnapshot  // 压缩时为 zlib，解压后必须是 32768 字节
125 × tagged Int32    // 本样本当前 public schema 的索引字段投影
0
```

快照前四字节与 wire CRC 相同，CRC 覆盖后续 32764 字节。实际两条报文的校验分别为 `0x49350930`、`0x53ef74dd`，都与独立 Python zlib.crc32 计算一致。目标序列化使用 `mp/playerdata.def`；private 路径使用 `mp/playersecuredata.def`。没有使用旧版 220/11 档案种子。

读取结果先有 tagged UInt64 UID（`0x00a5a370`），然后是上述参数但不含请求终止 0。写入任务不绑定结果数组，所以保存成功后发送零记录状态回执。

实现位于 `crates/protocol/src/lobby/profile.rs`、`crates/server/src/backend/profile/mod.rs`。解压限制固定为 32 KiB，拒绝 CRC 错误、压缩流尾随数据、膨胀和未知投影形状。只在完整校验后原子替换 `profiles/<title>/<UID>/public.bin`，保留客户端的精确序列化字节。测试覆盖重启读回、身份隔离及坏 CRC 不覆盖已存数据。完整游戏语义和索引字段还未解析；其他字段投影暂不接受。

## 80/230 初始玩家数据

请求构造器 `0x00a472b0`，调用者 `0x004a5080`/`0x004a5120`；参数仅为空 context String。结果绑定 `0x07bdc6f8`，对象大小约 `0x8010`，构造器 `0x00a4a5b0`、解码器 `0x00a4a6b0`。

80/230 返回一条绑定当前账号的结构记录；零库存形状为：UInt64 UID + String accountType、UInt32 余额数 0、UInt32 物品数 0、UInt32 权益对数 0。权益解码器 `0x00a4bd80` 从父对象继承用户身份；其计数虽然在内存中截为 UInt16，wire 仍是 UInt32。UID=-1 的 SDK sentinel 情况需要额外编码账户身份，已单独处理。

实际账号记录携带库存和余额，经验启用时读取已提交的武器 XP。初始 `80/230` 内嵌库存继承外层身份，`80/203` 独立库存逐条编码账户，不能混用。玩家初始化、公共档案上传／重登、有效私有档案和仓库均已有原生验证，见[档案与库存](PROFILE_INVENTORY_PROGRESS.md)。

## 生成公共文件

```sh
target/release/codol-server prepare-publisher \
  --output-dir client-work/server-data/publisher
```

输出文件必须不存在。游戏、账号、密钥及生成的 publisher 文件留在本机忽略目录；正常准备与独立诊断资料分开维护。
