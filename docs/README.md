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

当前入口：[游戏服分配与 UDP](GAME_TRANSPORT_PROGRESS.md)。登录、仓库、等级 70、房间等级锁、持久钱包和永久单件购买已接入；商城切换细节按用户要求暂缓。单客户端房间、训练赛单中文/人数与准备广播已恢复。训练准备后现在可进入地图加载界面并完成随机 P-224 握手，Rust 已解密真实 getchallenge；challenge 回复已补并通过模拟 UDP 集成测试，实际复测遇到 Wine/IIPS 启动阻塞；没有出生或 AI。

已实现 AUTH 测试票据、协议 210 大厅、UDP 地址发现、档案/文件、钱包与库存、购买原子保存、单人 channel、游戏分配/booking、目标版本的 SHA1/AES128 游戏通道。67 项 Rust、13 项 Python 以及 fmt/Clippy 通过。数据仍用隔离文件，无 SQLite。

下一次工作：

1. 复用现有克隆、prefix、publisher 和 profiles，不要重新初始化已有账号。
2. 先排查最新 Wine/IIPS 启动阻塞（采样和实验见当前入口），再从 game transport 的 getchallenge 接续：恢复目标 challenge/connect 消费者、身份绑定、gamestate 和场景出生；之后才做 Bot/剧情或双客户端。
3. 剧情首关没有先发 21/1，尚缺该路径的实时 security key；不得使用零密钥兜底。
4. AlphaBuild 更新中的 admission/连接结构可参考，但目标 3.3.3.3 的游戏加密已经证实不同，详见新文档。无参考 .git 元数据，不能把文件日期当作精确更新 diff。
5. 按验证阶段 Git commit；本地提交即可，没有要求推送。参考快照和运行抓包均在忽略目录，不能加入账号/密钥原件。

研究原件在仓库的上一级：`../CODOL/`、`../CODO_OfflineLauncher.exe`、`../Installer/`。362 个 `.ifs` 容器已核对文件名、大小、nifs 文件头并记录 SHA-256；容器内部块和资源完整性仍未知。没有可用的本地官方成功抓包，实验依赖客户端解析路径及可观察的状态变化。

工具提示：shell 未找到 `rg`，使用 `find`。Homebrew Rust/LLVM 存在链接错误，使用 sh tools/cargo.sh 选择 Rust 1.96.0。静态工具在 .venv。Ghidra 12.1.4 已解压到 .tools/ghidra，并编译了 Mac arm64 decompiler；已有目标函数结果在 research-output/ghidra/auth-targets.c，使用 tools/ghidra/DecompileTargets.java 导出。GameHub 10 已验证、11 未做客户端实验。OpenSSL 使用 /opt/homebrew/bin/openssl。Parallels Windows 11 Arm 已准备好。

后续每次研究更新 CLIENT_BASELINE.md 的证据、方法、地址类型和置信度；需求或技术路线改变时更新方案，并在此处更新已完成状态及下一步。先前的“双客户端优先”已被最新的“登录、背包、大厅优先”替代。
