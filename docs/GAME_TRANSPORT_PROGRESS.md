# 游戏服分配与 UDP 通道

更新：2026-10-07。目标仍为 CODOL 3.3.3.3，EXE SHA-256 `e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。

已实测：人机训练准备后，Rust 分配游戏端点、回应 mmbook，客户端进入“荒漠边陲”加载画面，完成 P-224 握手；Rust 成功验证并解密 `getchallenge`。当前停在等待连接，尚无 challenge 回复、connect、gamestate、出生、Bot AI 或完整对局。加载画面不能计为可玩。

## 分配与开局控制

1. `21/5 query=3` 是找同伴，实验端点返回空列表，客户端自行建单人 channel。
2. 准备后 `query=1`（仅 build 条件）是查询游戏服务器。返回真实已绑定的 UDP 端点，而非没有接收端的地址。
3. 查找结果比创建描述多 Blob[8] security ID 和一个 UInt32 基类字段；目标解码 `0xa55700 → 0x701db0`。地址为 37 字节 commonAddr，host UID 使用独立的 `1000001`。
4. 客户端向 host 发 `6/14`：UInt64 recipient、Blob（原始 LE32 文本长度 + ASCII 文本）。`mmbook <playlistVersion> <base64 ID> <0|1>` 回复 lobby kind=2、event=21（GLOBAL_INSTANT_MESSAGE），同样长度封装 `mmbook accepted`。目标事件枚举 `0xcd6d80`。
5. 随后收到 `mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0`。只保存元数据，不把收到的命令交给 shell/控制台执行。只有 booked + start requested 后才允许 INIT。

Admission 按 title/UID/登录 seed 绑定；security ID 随机生成，密钥来自该会话实时 `21/1` 描述中的 16 字节 security key。重试保持 ID；新登录、key 变化、退出/删除房间或到期撤销旧分配。有效期不超过票据与 930 秒中的较早者，最多 32 个 admission/peer。尚未在异常大厅断线时立即撤销，仍由上述条件清理。

剧情首关之前实测没有 `21/1`，目前明确返回 `game_allocation_requires_live_key`。后续需要恢复这个路径的密钥来源，不能用零密钥或 AUTH key 冒充。

## 3.3.3.3 握手与 DATA

| 报文 | 本轮观察 |
| --- | --- |
| NAT probe | 29 字节，type=13/version=2；回复 type=12 |
| INIT | 16 字节：6 字节 header、remote tag u16、security ID[8] |
| INIT_ACK | 38 字节；COOKIE_ECHO 原样回送，服务器逐字节验证 |
| COOKIE_ECHO | 189 字节：header[6]、INIT_ACK[38]、commonAddr[37]、ID[8]、public[100] |
| COOKIE_ACK | 114 字节：header[6]、server public[100]、ID[8] |
| DATA | type/version/tag/counter 共 6 字节，MAC[8]、encrypted length u16、加密区、可选明文尾部 |

ECDH 曲线 secp224r1。公钥为 LibTomCrypt DER（包含 bit-string、curve-size=28、X/Y 整数）补零到 100 字节。验证坐标和曲线；服务器每次新 INIT 随机生成私钥，相同 INIT/cookie 重传复用原回复。

目标 `0xa7af90` 的 KDF：`K = SHA1(sharedX[28] || securityKey[16]) || 00000000`，24 字节。`0xb2ced0/0xb2cf50` 的算法注册字符串 `0xbf16f4` 为 sha1，摘要长度 20。AES 使用 K 的前 16 字节，CTR 计数为小端。

目标 DATA 解码 `0xa7fda0`：

- IV = `SHA1(LE32(完整序号) || LE16(目标 tag))[0..16]`。
- MAC = `HMAC-SHA1(K[24], wire[0..6] || wire[14..])[0..8]`。**长度字段及明文尾部均参与校验**。
- 只解密 encrypted length 指定的字节，尾部认证但不加密。
- 16 位 wire counter 还原成完整 32 位序号；目标 `0xb2da20` 为 32 包防重放窗口。认证失败不消耗序号，发送计数耗尽后拒绝继续发送。

这与 AlphaBuild 1.1.1.13 的 Tiger/AES-192/旧 IV 规则不同。之前自洽的收发 roundtrip 测试不足以证明互通，因此新增 Python hashlib/hmac + PyCryptodome 独立生成的合成向量，覆盖跨 AES 块、明文尾部及完整序号。

## 实验记录

- `client-1791297085529`：分配接受，客户端反复发送未实现的 6/14，定位 mmbook。
- `client-1791299964181`：booking/mmstart 接受，加载地图并发出 NAT/INIT。
- `client-1791305078332` / `client-1791305698641`：COOKIE_ACK 接受，旧 DATA 算法校验失败；随机与诊断固定私钥都失败。固定私钥不能解决版本差异，已移除运行时用法。
- 离线核对最后一次抓包：上述 SHA1 KDF + 含长度字段的 MAC 匹配；AES128 解密得到 `getchallenge 0 "<当前 UID 的十六进制>"`，以 NUL 结束。
- **`client-1791307705995`**：新 Rust + 随机 P-224 私钥真实验证，前 30 个 DATA 全部 MAC 通过，无握手错误，明文包括 getchallenge 与 2 字节连接维护数据。截图 `ui-1791307737897.png` 为“荒漠边陲 / 等待连接”。后续仍因未回复 challenge 超时。

完整原始包、私钥相关诊断材料和账号数据只在忽略目录 `research-output`/`client-work`，不要加入 Git 或在日志输出密钥。

## 运行与下一步

```sh
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --seconds 900 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun --rust-game \
  --local-services --trust-lab-cert --quiet-network
```

`--rust-game` 映射 `serve-auth --game-bind 127.0.0.1:39004`，要求本地 services 与 lobby；当前只允许 loopback。抓包在本次实验的 `auth-captures/auth-*/game`，event.json 区分 MAC 通过、错误及明文长度。

下一步从目标的 challenge/connect 消费者恢复回复及身份绑定，再处理 gamestate/snapshot、场景出生与 AI；多客户端成员加入、定向广播和每席 admission 尚未实现。先核实目标消费者，不能直接用旧参考的 protocol/shortversion 常量。

本阶段验证：fmt、Clippy、66 项 Rust、13 项 Python 通过。新增测试覆盖独立加密向量、防重放/乱序/计数耗尽、P-224 cookie/key exchange、分配过期与密钥轮换、mmbook 身份隔离、真实 UDP admission gate 与 INIT 重传。
