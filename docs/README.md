# 接续入口

最后更新：2026-10-07。本仓库用于 CODOL 3.3.3.3 的 Rust 服务端重建。

- [实施方案](REVERSE_ENGINEERING_PLAN.md)：范围、技术路线、Mac 运行策略、模块、里程碑及验收标准。
- [已发现的事实](CLIENT_BASELINE.md)：样本哈希、文件偏移、配置、资源清单、环境与未验证假设。
- [启动链与配置读取](BOOTSTRAP_FINDINGS.md)：CIL 与原生指令的静态结论、RVA/VA 和下一步交叉引用。
- [工程基础与运行手册](FOUNDATION.md)：已经实现的能力、构建/测试/观测命令、输出格式与限制。
- [GameHub 动态调试](GAMEHUB_DEBUG.md)：真实 ClientHello、HTTPS AUTH 请求、title_id 对照实验、运行工具和响应逆向入口。
- [AlphaBuild 审阅](ALPHA_BUILD_REVIEW.md)：用户提供的 1.1.1.13 实现、与 3.3.3.3 的交叉核对和移植依据。
- [2.6.9.7加载文档对照](LOADING_2697_REVIEW.md)：本版IFS明文清单、M4跨包资源与模型表装配函数的核对。
- [AUTH 与大厅通道](AUTH_LOBBY_PROGRESS.md)：Rust 认证票据与协议 210 的真实客户端验证、UDP 地址发现、业务帧解密。
- [业务初始化与文件服务](SERVICE_INITIALIZATION.md)：任务 FIFO、时间、存储格式与早期公共配置失败。
- [公共配置与玩家初始化](PUBLISHER_FILES.md)：配置解密与解析、80/230 和档案格式。
- [新玩家引导与只读调试](ONBOARDING_PROGRESS.md)：真实公共档案上传/重登、初始化与只读状态探针。
- [私有档案与初始库存](PROFILE_INVENTORY_PROGRESS.md)：目标 233/19 定义、8 件库存和仓库界面。

用户最新要求：先恢复登录、背包和大厅，然后推进本地房间/人机与双客户端最小对局。优先使用 GameHub Wine/Proton 在 Mac 上运行，Parallels Windows 保留作对照；服务端使用 Rust。

当前入口：[玩家出生、输入与装备](PLAYER_BOOTSTRAP_PROGRESS.md)。阿富汗场景/HUD可见，用户已确认持续鼠标转向。commandTime已与快照时钟分离；输入base按ACK选择，组合M4、弹药、entity0已下发。射击后弹匣不会再被快照重置30；换弹状态已实现，实机结果见该文档。持枪模型/瞄准仍异常，尚无权威移动、碰撞、命中和AI。

[客户端启动恢复](CLIENT_STARTUP_RECOVERY.md)记录了两次同一IFS文件对象阻塞。当前prefix为 `wine10-play-20261007`，只读探针单独用 `wine10-probe-build-20261007` 编译；保留已有账号、库存和钱包。已结束实验的清理状态以对应experiment.json为准，运行中的实验有900秒期限。

检查结果：95项Rust、19项Python及fmt/Clippy通过。普通单元测试不代替画面与操作验收。

下一次工作：

1. 继续核对第一人称DObj、XAnim树与模型骨骼绑定。idle0x400→0x401实验没有修复姿态，已撤回；不要重复以动画编号猜测作为修复。注册定义中有viewmodel_m4a1tech动画名，尚不能证明实际绑定正确。
2. 补全服务端ADS、换弹中断、武器事件及射击状态连续性；随后接入地图碰撞与权威位移。每帧固定出生坐标不是移动实现；不把客户端预测动作当作服务器完成。
3. 复用已验证native方向、NCS CRC、时钟配置和ACK输入base。改PS字段时核对目标版本的235项字段表，不能照搬Alpha布局。
4. 复用现有客户端与业务数据，不重新初始化账号。商城切换暂缓；剧情首关仍缺无21/1路径的security key。
5. 每个可验证阶段Git commit；没有要求推送。原始抓包、资源、账号和密钥保留在忽略目录。

研究原件在仓库的上一级：`../CODOL/`、`../CODO_OfflineLauncher.exe`、`../Installer/`。362 个 `.ifs` 容器已核对文件名、大小、nifs 文件头并记录 SHA-256；容器内部块和资源完整性仍未知。没有可用的本地官方成功抓包，实验依赖客户端解析路径及可观察的状态变化。

工具提示：shell 未找到 `rg`，使用 `find`。Homebrew Rust/LLVM 存在链接错误，使用 sh tools/cargo.sh 选择 Rust 1.96.0。静态工具在 .venv。Ghidra 12.1.4 已解压到 .tools/ghidra，并编译了 Mac arm64 decompiler；已有目标函数结果在 research-output/ghidra/auth-targets.c，使用 tools/ghidra/DecompileTargets.java 导出。GameHub 10 已验证、11 未做客户端实验。OpenSSL 使用 /opt/homebrew/bin/openssl。Parallels Windows 11 Arm 已准备好。

后续每次研究更新 CLIENT_BASELINE.md 的证据、方法、地址类型和置信度；需求或技术路线改变时更新方案，并在此处更新已完成状态及下一步。先前的“双客户端优先”已被最新的“登录、背包、大厅优先”替代。
