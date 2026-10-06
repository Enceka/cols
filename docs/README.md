# 接续入口

最后更新：2026-10-06。本仓库用于 CODOL 3.3.3.3 的 Rust 服务端重建。

- [实施方案](REVERSE_ENGINEERING_PLAN.md)：范围、技术路线、Mac 运行策略、模块、里程碑及验收标准。
- [已发现的事实](CLIENT_BASELINE.md)：样本哈希、文件偏移、配置、资源清单、环境与未验证假设。
- [启动链与配置读取](BOOTSTRAP_FINDINGS.md)：CIL 与原生指令的静态结论、RVA/VA 和下一步交叉引用。
- [工程基础与运行手册](FOUNDATION.md)：已经实现的能力、构建/测试/观测命令、输出格式与限制。
- [GameHub 动态调试](GAMEHUB_DEBUG.md)：真实 ClientHello、HTTPS AUTH 请求、title_id 对照实验、运行工具和响应逆向入口。
- [AlphaBuild 审阅](ALPHA_BUILD_REVIEW.md)：用户提供的 1.1.1.13 实现、与 3.3.3.3 的交叉核对和移植依据。
- [AUTH 与大厅通道](AUTH_LOBBY_PROGRESS.md)：Rust 认证票据与协议 210 的真实客户端验证、UDP 地址发现、业务帧解密。
- [业务初始化与文件服务](SERVICE_INITIALIZATION.md)：任务 FIFO、时间、存储格式与早期公共配置失败。
- [公共配置与玩家初始化](PUBLISHER_FILES.md)：配置解密与解析、80/230 和档案格式。
- [新玩家引导与只读调试](ONBOARDING_PROGRESS.md)：**当前接续入口**。真实公共档案上传/重登、商城初始化完成证据、余额/附件列表，以及下一步物品和私有档案恢复。

用户最新要求：先恢复登录、背包和大厅；双客户端最小对局是第二阶段。优先使用已安装的 GameHub Wine/Proton 兼容层在 Mac 上运行；Parallels Windows 已准备好，保留作对照。服务端使用 Rust，游戏客户端沿用提供的 Windows 程序。

目前已完成 Rust TLS/HTTP AUTH、动态本地测试票据、每票据独立会话密钥、协议 210 大厅认证、AES/HMAC 业务帧及 UDP v3 地址发现。真实 GameHub 客户端已进入存储、商城及时间初始化阶段。新增可选本地服务模式，按请求顺序回复时间、用户/公共文件任务；未知任务返回明确失败。用户文件持久化通过重启读回和身份隔离测试。

M0、认证通道、公共配置、the.json 和公共档案的真实上传/重登已有动态证据；第一阶段整体未完成。最新界面可停在无忙碌弹窗的新玩家引导，目录状态为完成，但目录和初始物品仍为空，完整大厅/背包未验证。私有档案仍未实现。运行的是本地测试身份，不是腾讯账号认证。全部实验使用独立客户端克隆和 Wine prefix，临时 CA 已恢复；原件保持不变。用户要求按验证阶段及时 Git commit，初始基线为 1d6a9e9。

用户先暂停并提供 AlphaBuild，完成审阅后再次要求继续。当前已据此实现票据，并独立恢复了目标版本新增的协议 210；参考目录仍保持原样。

下一次工作从这些步骤接续：

1. 阅读 ONBOARDING_PROGRESS.md，复用现有客户端、prefix、publisher 与 public.bin；不要清空已经真实保存的档案。启动 AUTH+LSG+STUN 及 --local-services。
2. 从目标资源/内存读取物品表和 public/private StructuredDataDef，确定起始装备、引导完成条件与私有档案布局。公共档案透明上传/重登已经验证，不必重做票据或 FIFO 调查。
3. 目标大厅使用协议 210：HKDF-SHA1、AES-128-CBC、HMAC-SHA1。不要照搬 AlphaBuild 的旧大厅 3DES 帧；3DES 目前用于 AUTH 票据。
4. 80/70、80/107 已支持空测试目录并达到客户端完成状态；93/1 返回已消费的零余额，80/173 返回无附件。非空商品、角色/武器与权益仍待目标资源支持；不要将空目录称作完整背包。
5. 用 ReadClientState 只读探针观察已知状态，避免 WineDbg 附加异常。大厅研究总时限现为 330 秒。装备修改→上传→重登保留仍需验收；当前通过 41 项 Rust、5 项 Python 测试，并继续按阶段 Git commit。

研究原件在仓库的上一级：`../CODOL/`、`../CODO_OfflineLauncher.exe`、`../Installer/`。362 个 `.ifs` 容器已核对文件名、大小、nifs 文件头并记录 SHA-256；容器内部块和资源完整性仍未知。没有可用的本地官方成功抓包，实验依赖客户端解析路径及可观察的状态变化。

工具提示：shell 未找到 `rg`，使用 `find`。Homebrew Rust/LLVM 存在链接错误，使用 sh tools/cargo.sh 选择 Rust 1.96.0。静态工具在 .venv。Ghidra 12.1.4 已解压到 .tools/ghidra，并编译了 Mac arm64 decompiler；已有目标函数结果在 research-output/ghidra/auth-targets.c，使用 tools/ghidra/DecompileTargets.java 导出。GameHub 10 已验证、11 未做客户端实验。OpenSSL 使用 /opt/homebrew/bin/openssl。Parallels Windows 11 Arm 已准备好。

后续每次研究更新 CLIENT_BASELINE.md 的证据、方法、地址类型和置信度；需求或技术路线改变时更新方案，并在此处更新已完成状态及下一步。先前的“双客户端优先”已被最新的“登录、背包、大厅优先”替代。
