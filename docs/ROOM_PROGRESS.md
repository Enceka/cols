# 本地房间与 PVE 剧情

更新：2026-10-06。当前接续入口。用户已确认可以进入房间，最新要求是**优先 PVE 剧情模式**；同时修复准备后不能取消的问题。购买/武器等级扩展暂停，前一阶段见 [PURCHASE_PROGRESS.md](PURCHASE_PROGRESS.md)。

## 已实现

| 任务 | 当前本地行为 |
| --- | --- |
| 71/2 | 名称检查：Bool sanitized=false + 原 UTF-8 字符串，长度有界、拒绝空串/控制字符 |
| 11/13 | 创建独立 room channel；当前会话重试复用 ID |
| 11/2 | 校验所属会话后退出、删除 channel |
| 21/1 | 校验房主 UID、channel 引用和目标描述格式，签发 8 字节会话 ID |
| 21/12 | 校验已签发 ID 和 channel 后更新描述 |
| 21/3 | 删除当前账号所属会话注册 |
| 11/15 | 返回当前单个房主成员 |
| 11/4 | 校验当前单人房间所属关系与批量消息边界，接收并确认；没有其他成员可转发，保留有界队列供后续恢复 |

channel 按 title/UID/session 绑定，最多 128 个，随票据过期清理。描述最多 1024 字节，房间消息批次最多 16 KiB / 64 个子消息，保留最近 32 批。密钥只在当前会话描述内保留，不输出日志或 Git；fixture 使用虚构身份、零密钥。

实际多客户端加入、消息转发、开局分配、游戏状态、AI 和剧情脚本尚未实现。当前 11/4 的接收确认需要继续验证“准备→取消”UI 行为；不能据此声称完整房间同步。

## 真实客户端进展

- client-1791285716279：创建提示“网络环境不稳定”，没有发 11/13；反复失败的是名称检查 71/2。此提示不是已证明的 NAT 故障。
- client-1791287174783：名称回包接受，随后 11/13 接受并发出 21/1；当时 21/1 未实现，触发未知错误和 11/2 退出。
- **client-1791287801832**：11/13、21/1 成功；截图 ui-1791287960854.png 为真实房间：房主 steamuserClient、1/12、地图“安全屋”、战术团队竞技，提示还需 3 名玩家。用户确认已进入房间。
- 此时 11/4 仍失败，用户反馈准备后不能取消。已补接收确认，下一实验验证。

## 目标代码证据

71/2 builder=0xa48640，调用者 0x4add80，结果构造器 0xa520e0，vtable 0xc078d8，解码器 0xa52180。解码 Bool 到 +8，再读 String；0x6aa540 将 Bool 命名为 sanitized，所以未修改的本地名称返回 false。

11/13 builder 0xa4c940，由 0x6c9f30/0x6c9e90 绑定输出。channel 构造器 0xa55970，解码器 0xa55a40。记录：UInt64 ID、UInt32 成员数、UInt32、UInt64 管理者、Bool、Bool、UInt32。当前 1 名成员、认证 UID 为管理者，其余零默认。成员记录 0xa55c20：UInt64 UID、String[64]、UInt32 flags。

前台接口指针 0x078b0708，实测 vtable=0xc7eba4，CreateRoom=0x6e2650，最终调用 0x6c9f30。

21/1 builder=0xa56cd0，更新 builder=0xa56da0。实测 231 字节请求（含服务/任务），与 AlphaBuild 的旧描述不同：

1. Blob[37] 地址；UInt32；UInt32 最大人数；String 房间名；UInt64 房主；String 房主名。
2. 3×Int32；UInt32；3×Int32；Blob[16] 会话密钥。
3. 11×Int32；UInt64 channel ID；2×Int32；终止 0。

未知整数字段保留，不臆断为已完成的地图/玩法语义。更新前缀为 Blob[8] 会话 ID 和 UInt32，后接同样描述。11/4 请求是 UInt64 channel、Blob 批次、UInt32 delivery（实测 0/1）；批次中每条为原始 UInt32 长度及消息体。

## 后续验收顺序

1. 用新增 11/4 确認验证准备→取消→重新准备，保留真实请求和截图。
2. 进入剧情模式，选择第一关，确认它是否复用本地 room 或走另一套匹配/分配接口。
3. 恢复单客户端开局分配与进图；准确记录地图名、模式、地址、会话 ID、资源和握手依赖。
4. 区分成功加载场景、实际出生、剧情触发/敌人 AI；只有最后两项有证据后再评估扩展关卡或多人 PVE。

不将 PVP 房间里的最低 4 人限制简单改成 1 来冒充人机实现。当前尚未确认目标客户端保留的本地 Bot/监听服能力。

55 项 Rust、13 项 Python 测试通过（名称、隔离、重试、过期、会话绑定、描述截断、消息边界），另有 fmt/Clippy。完整实验、资源和账号数据留在忽略目录。

## 快速开始列表、人机标记和中文编码

用户指出“快速开始游戏”应有人机训练入口。已确认服务器下发的最小赛单分类错误：catinfo 的第二个数是页面筛选值，0=PVE、1=PVP；第三个数控制展示（0=普通、1=featured、2=隐藏）。原 `catinfo 1 0 0` 导致 Local Research 出现在 PVE 页、PVP 为空。

目标 category 基址 0x0783faa0、步长 0x454；id/page/presentation 分别为 +0/+4/+8，name=+12，playlist 数=+0x44c。UI getter 0x6acab0 明确比较 +4 与请求页面、排除 presentation=2。目标 parser=0x6111d0；`combattraining` 在 0x611ed4 将 playlist flags 置 0x10。旧参考也有“新手训练”“人机训练”赛单，但未整份复制旧版文件。

版本 2 新增 PVP 的人机训练和团队竞技分类，均使用目标存在的 mp_afghan/war；还未实现 AI/对局分配。用户发现中文乱码，已改为版本 3：仓库中的源配置保持 UTF-8，`prepare-publisher` 使用 GBK 编码生成 wire 文件，拒绝无法表示的字符。独立 Python GBK 字节比对已通过；不能直接把 UTF-8 源文件复制到服务数据目录。

实际 publisher 旧文件保存在 research-output/playlist-before-training.info 和 playlist-utf8-v2.info；新生成文件在 publisher-gbk-v3。当前正在进行中文显示验收。

PVE 路径已有新发现：client-1791288403789 的剧情首关“身份不明”进入“战斗开始”提示，产生 11/13→21/5，没有先发 21/1。因此 11/4 接收校验改为只要求属于当前 channel，不能把已有游戏会话注册当作前提。开局 21/5 仍未实现。

为了连续 UI 调试，harness 上限现为 900 秒、lobby 总时限 930 秒，最多 1024 个业务帧；1 MiB 请求预算和 Wine 日志预算仍保留。测试更新为 56 项 Rust、13 项 Python，包含 GBK 回归向量。

## 人机训练的人数错误与直接开局要求

用户截图显示训练进入房间后为 1/0，并提示“玩家数量超出上限”；期望训练自动进入游戏。版本 4 补充目标人数规则：0x47a57f 注册名 `1011`，写入 0x01a323c4（最少人数）；0x47a59b 注册名 `3265`，写入 0x01a32368（最多人数，默认 0）。0x6e26e3..0x6e2719 通过 0x6137c0 执行赛单规则，再读取两个 dvar 的 +12 current 值。不能使用旧参考的上限编号 2481。

人机训练规则为 1011=1、3265=12，团队竞技为 1011=2、3265=12。`dw_autoStartGame` 在 0x6ca862 注册时默认 true（0x078a04fc），无须臆造开关。修复人数并不等于已能直接进图：client-1791291071883 的训练流程先发送 query=3 的 21/5（8 个 Int32 条件），失败后进入 channel；仍需要恢复真正的服务器分配。客户端内部建 channel 可以是过渡状态，不能把停在房间 UI 当作训练开局完成。

当前验证实验为 client-1791292128672，加载 GBK 赛单版本 4。旧文件备份 playlist-before-player-limits.info，生成物在 publisher-gbk-v4。

该实验 ui-1791292237201.png 已确认中文“人机训练”和人数 1/12，原 1/0 问题消失。但只 ACK 准备请求仍不足以推进状态，客户端连续发送相同 ready 消息，不能把这种结果记为取消准备成功。

## 房间广播与当前开局阻塞点

新增 lobby kind=2 推送，事件号 7。目标外层分派 0xa43ba0 的 jump table 0xa43df0 将 kind 2 交给 0xa42db0；事件 7/8 再到 0xa3d5a0。body 顺序为 UInt32 event=7、UInt64 sender、String[64] sender name、UInt64 channel、Blob、UInt32 delivery。该目标 Blob 上限是 1024 字节；批次按完整子消息切块，拒绝单条超长。

对当前单人 channel 回送本人消息；之后的多成员广播仍待实现。推送复用 S2C 加密序号，不消耗 RPC transaction ID。新增传输测试验证 ACK→push→下一 RPC 的序号和 FIFO 不串位。

client-1791294035140：前三条 11/4 均成功并各发送一次推送。ready 后出现新的 21/5，随后客户端因未实现的游戏服查询返回错误，发送 21/3、11/2 清理并回大厅。用户报告“服务器繁忙，请稍后”，截图 ui-1791294155906.png 一致。此处是分配服务缺失，不能声称已经自动开局或把它解释为客户端进程崩溃。

## 下一步直接从游戏服分配接续

同一实验的 21/5 有两种明确阶段：packet-039 是 query=3、offset=0、maxResults=50，带 8 个 Int32 条件；准备广播之后 packet-048 是 query=1、offset=0、maxResults=10，仅一个 Int32 build 值 -688906341。前者是匹配入口，后者是准备后的游戏服查询。不能将两者都当成创建 channel，也不能拿固定空成功宣称开局恢复。

目标 SDK builder=0xa56f50；游戏调用点 0x6d44bb、0x6d4953。结果数组 0x0c89eb20，初始化器 0xb49f25，主 vtable=0xc78450，反序列化器=0x701db0，序列化器=0x701b60。基类解码 0xa55700：Blob 地址、Blob[8] session ID、3×UInt32；然后是创建描述从房间名起的同样尾部。请求基类 0xa554c0 仅有地址和 2×UInt32，因此查找结果比创建描述多 session ID Blob 和一个 UInt32，不能原样回显请求。

静态导出已保留：find-session-result.c、matchmaking-record.c、matchmaking-base.c（research-output/ghidra）。接续先确认额外 UInt32 的人数语义及 query 条件，再接 Rust 游戏端点和真实会话分配，采集 UDP 握手。当前工程尚无能够承载地图、出生和 AI 的 Rust 游戏进程，所以自动进图仍未完成。不要通过返回一个没有接收端的假地址掩盖此缺口。

本阶段实现已提交 a4095c2（训练分类、GBK、人数字段、本人房间广播与传输 FIFO 回归）；前序房间基础 c6ed48a、购买 d06f197。最新测试 56 Rust + 13 Python 全部通过。最后一次客户端实验已按 900 秒时限清理结束。

## 2026-10-07 更新：已越过分配与握手

当前进度转到 [GAME_TRANSPORT_PROGRESS.md](GAME_TRANSPORT_PROGRESS.md)：真实 UDP 端点分配、mmbook/mmstart、随机 P-224 握手及目标版本 DATA 校验/解密已验证，训练进入荒漠边陲加载画面并发出 getchallenge。原“服务器繁忙”分配失败已越过；当前阻塞变为尚未处理 challenge/connect 与世界状态，因此仍不能出生/打人机。剧情首关的 key 来源和多成员房间仍待恢复。
