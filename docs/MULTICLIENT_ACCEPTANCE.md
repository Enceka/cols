# 双客户端与多人联机验收

2026-10-15，固定 CODOL 3.3.3.3 客户端，EXE SHA-256：
`e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b`。

本机已实测两个独立游戏窗口并存。完整多人联机验收未通过：同机原生认证身份冲突，实际房间分页查询被拒绝，参与者装配报文未支持，共享对局世界也未接入。没有取得两个原生客户端同房准备、同局互相可见、伤害／计分同步或共同结算的证据。

| 验收项目 | 当前结果 | 证据范围 |
| --- | --- | --- |
| 独立客户端目录与 Wine prefix 双开 | 通过 | 两个原生进程与窗口，PID 38554／38793，窗口 2830／2839 |
| 不同认证身份同时在线 | 未通过 | 不同 `-q` 参数仍签发同一 UID；大厅连接被后续登录撤销 |
| 原生创建房间 | 单端通过 | A 的画面为 1/12；只读探针确认房主、房间成员数与 SDK 频道成员数均为 1 |
| 原生公共房间列表 | 未通过 | 实际 `21/13 query=4` 请求返回 `unsupported_browser_criteria` |
| 查询、入房、准备／取消、退房与断线通知 | 自动验证通过 | 两个合成身份的业务与真实 TCP 测试；不代表双原生客户端通过 |
| 真人参与者角色／武器装配 | 未支持 | 合法 type18 记录被参与者白名单拒绝 |
| 多真人共享开局与世界同步 | 未支持 | 分配返回 `shared_game_world_not_available`、错误码 4；连接各自持有 `LocalWorld` |

双开使用两套独立可写目录、两套 Wine prefix，并连接同一个隔离 Rust 后端。AUTH、LOBBY 与 IIPS 观察端点为 39202、39201、39203，服务资料为独立副本。启动器的同目录锁保护连接配置、证书和恢复记录，不应解除该锁来双开。当前 `native-launch` 每次管理自己的后端，正式多人启动还需要连接共享服务器的运行方式。双窗口并存尚不能证明两场景同时运行的性能，也不能证明局域网或公网可达。

原生资源加载在 `IIPS/IIPSDownload/ff_init_common_gfx_V43.ifs` 的属性／打开路径阻塞。测试目录使用两份独立可读资源对象后，两个客户端均完成原生初始化与协议 210 认证。该资源长度为 26504032，SHA-256 为
`a4bac3f12ef090773ee5fc5017e5f88dae600f2d81005e0f67257c02b78ff667`，与已有基线一致。只替换测试目录的资源路径；未改写 EXE／DLL 或原资源对象。APFS／Wine 阻塞的底层成因仍未确定。

两端原生认证分别使用 `-q 10001` 与 `-q 10002`。AUTH 记录只有一个不同 UID，大厅日志明确出现 `lobby login was superseded`。A 的画面显示大厅服务中断后重新建立，B 的画面显示 `[100]无法连接大厅服务`。不同 `-q` 参数、昵称或本地玩家存档目录不能在当前验收中充当不同联网身份。原始 UID、认证票据和密钥只保存在忽略目录；公开摘要只比较 UID 的哈希。

“玩 → 多人竞技 → 查找房间”实际发送：

```text
service=21, task=13, query=4, initial=true, pageSize=30
criteria=[-688906341, 0, 16, 0]
```

格式与 MAC 验证成功。`backend/rooms/directory.rs` 只接受第三项为 2，因此页面空列表不是成功查询。第三项 16 的完整筛选语义仍需恢复；不通过忽略条件或伪造记录将该项标记为通过。

`backend/rooms/mod.rs` 在房间真人数不为 1 时拒绝游戏分配；`transport/game/connection/mod.rs` 中每个连接拥有独立世界。多个 UDP peer 或独立 ACK 不能形成共享对局。type18 在 `protocol/room/messages.rs::validate_participant` 中被拒绝，原生 Bot 装配验收不能替代真人装配验证。

自动验证使用独立检出 `46cac01`，并复制当前 `room_browser_acceptance.rs` 到该检出。两项 TCP 房间测试、七项房间目录测试、一项重复登录测试通过；验收例子的七项查询／入房／限制检查通过。JSON 的 `passed=true` 表示复现这些预期行为，包括失败分支；`multiplayer_acceptance_passed=false` 明确表示多人验收未通过。验收例子通过 rustfmt 和独立检出的 Clippy 检查。

```sh
sh tools/cargo.sh test --locked -p codol-server room_transport_tests
sh tools/cargo.sh test --locked -p codol-server backend::rooms::directory_tests
sh tools/cargo.sh test --locked -p codol-server newer_login_revokes_old_ticket
sh tools/cargo.sh run --locked -p codol-server --example room_browser_acceptance -- \
  research-output/multiplayer-boundaries.json
```

原生测试使用冻结的本地 release 二进制，不从二进制推断 Git revision。主工作区整体编译曾因缺少 `PreparationHudSnapshot` 失败，工作区包含其他未完成改动；隔离检查不代表主工作区全量检查通过。

本机证据位于 `research-output/multiclient-acceptance-20261015/`：

- `native-identity-proof.json`：两个原生窗口、不同 `-q` 参数、同 UID 与重复登录结果。
- `native-browser-proof.json`：实际分页条件、页大小及失败 handler。
- `multiplayer-boundaries.json`：七项隔离检查与多人未通过标志。
- `resource-recovery.json`：测试资源摘要及隔离范围。
- `native-1791996058597120000/`：认证／大厅采集、截图、单端开房探针与退出 manifest。
- `stable-room-transport-tests.log`、`stable-room-directory-tests.log`、`stable-identity-tests.log`：独立检出的验证结果。
- `acceptance-clippy.log`：验收例子的 Clippy 检查。

测试游戏窗口与自建后端已结束，五项临时连接配置／证书已按 manifest 恢复。未向原有游戏会话发送终止命令。资源准备辅助进程 33518 在文件系统调用中阻塞，发送 SIGTERM／SIGKILL 后仍未被系统回收；保留现场，不重复读取故障对象。

多人功能完成还需要建立两个原生客户端可用的不同联网 UID，实现实际分页筛选条件，接入真人装配、队伍与 Bot 席位策略，再按房间持有一个权威世界与时钟，为每位玩家保留独立输入、加密、ACK 和快照。最终用两个原生客户端验证同房准备、同局移动／伤害／计分、断线、结算返回和再开局。
