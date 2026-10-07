# 接续入口

最后更新：2026-10-07。本仓库用于 CODOL 3.3.3.3 的 Rust 服务端重建。

- [实施方案](REVERSE_ENGINEERING_PLAN.md)：范围、技术路线、Mac 运行策略、模块、里程碑及验收标准。
- [已发现的事实](CLIENT_BASELINE.md)：样本哈希、文件偏移、配置、资源清单、环境与未验证假设。
- [启动链与配置读取](BOOTSTRAP_FINDINGS.md)：CIL 与原生指令的静态结论、RVA/VA 和下一步交叉引用。
- [工程基础与运行手册](FOUNDATION.md)：已经实现的能力、构建/测试/观测命令、输出格式与限制。
- [GameHub 动态调试](GAMEHUB_DEBUG.md)：真实 ClientHello、HTTPS AUTH 请求、title_id 对照实验、运行工具和响应逆向入口。
- [AlphaBuild 审阅](ALPHA_BUILD_REVIEW.md)：用户提供的 1.1.1.13 实现、与 3.3.3.3 的交叉核对和移植依据。
- [AUTH 与大厅通道](AUTH_LOBBY_PROGRESS.md)：Rust 认证票据与协议 210 的真实客户端验证、UDP 地址发现、业务帧解密。
- [业务初始化与文件服务](SERVICE_INITIALIZATION.md)：任务 FIFO、时间、存储格式与早期公共配置失败。
- [公共配置与玩家初始化](PUBLISHER_FILES.md)：配置解密与解析、80/230 和档案格式。
- [新玩家引导与只读调试](ONBOARDING_PROGRESS.md)：真实公共档案上传/重登、初始化与只读状态探针。
- [私有档案与初始库存](PROFILE_INVENTORY_PROGRESS.md)：目标 233/19 定义、8 件库存和仓库界面。

用户最新要求：先恢复登录、背包和大厅，然后推进本地房间/人机与双客户端最小对局。优先使用 GameHub Wine/Proton 在 Mac 上运行，Parallels Windows 保留作对照；服务端使用 Rust。

当前入口：[快照与输入最新进展](SNAPSHOT_PROGRESS.md)。真实客户端已进入阿富汗地图并显示场景/HUD，状态10；快照、native ACK、输入与可靠命令接收持续推进。地图出生坐标有资源依据。[Native通道](NATIVE_CHANNEL_PROGRESS.md)及[网络常量CRC](NET_CONSTANTS_CRC.md)记录前序基础。武器、完整玩家实体、权威移动、碰撞和AI尚未实现，不能视为可玩的对局。

[客户端启动恢复](CLIENT_STARTUP_RECOVERY.md)记录了 IIPS 故障文件打开阻塞及副本修复。现使用 `--prefix-name wine10-recovered-20261007`，保留已有账号和持久数据。gamestate 需显式 `--game-state` 启用，复现命令见最新 native 文档。

最新实验已停止并恢复临时CA。进图需使用带地图出生点的准备文件及 `--snapshot-probe`，完整复现命令见快照文档。

检查结果：87项Rust、17项Python及fmt/Clippy通过。

下一次工作：

1. 从最新 native 文档的 8/13 字节包头和两种 ACK 接续，不能沿用早期混淆方向的 prototype。
2. 从已渲染场景接续local client/entity、装备与真实移动。235项目标字段表已提取；先确认基线ACK，再使用普通flags0快照。输入默认base只支持当前零角度/武器/otherFlags；修改这些字段时同步升级输入base。不要重新猜CRC或照搬Alpha布局。
3. 复用客户端、恢复后的 prefix、publisher、profiles；不要重新初始化账号。商城切换暂缓；剧情首关尚缺无 21/1 路径的 security key。
4. 每个可验证阶段 Git commit；没有要求推送。原始抓包、资源、账号和密钥保留在忽略目录。

研究原件在仓库的上一级：`../CODOL/`、`../CODO_OfflineLauncher.exe`、`../Installer/`。362 个 `.ifs` 容器已核对文件名、大小、nifs 文件头并记录 SHA-256；容器内部块和资源完整性仍未知。没有可用的本地官方成功抓包，实验依赖客户端解析路径及可观察的状态变化。

工具提示：shell 未找到 `rg`，使用 `find`。Homebrew Rust/LLVM 存在链接错误，使用 sh tools/cargo.sh 选择 Rust 1.96.0。静态工具在 .venv。Ghidra 12.1.4 已解压到 .tools/ghidra，并编译了 Mac arm64 decompiler；已有目标函数结果在 research-output/ghidra/auth-targets.c，使用 tools/ghidra/DecompileTargets.java 导出。GameHub 10 已验证、11 未做客户端实验。OpenSSL 使用 /opt/homebrew/bin/openssl。Parallels Windows 11 Arm 已准备好。

后续每次研究更新 CLIENT_BASELINE.md 的证据、方法、地址类型和置信度；需求或技术路线改变时更新方案，并在此处更新已完成状态及下一步。先前的“双客户端优先”已被最新的“登录、背包、大厅优先”替代。
