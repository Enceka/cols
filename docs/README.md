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
- [公共配置与玩家初始化](PUBLISHER_FILES.md)：**当前接续入口**。配置解密与解析、真实文件重登、80/230、公共档案保存，以及新玩家引导界面的剩余问题。

用户最新要求：先恢复登录、背包和大厅；双客户端最小对局是第二阶段。优先使用已安装的 GameHub Wine/Proton 兼容层在 Mac 上运行；Parallels Windows 已准备好，保留作对照。服务端使用 Rust，游戏客户端沿用提供的 Windows 程序。

目前已完成 Rust TLS/HTTP AUTH、动态本地测试票据、每票据独立会话密钥、协议 210 大厅认证、AES/HMAC 业务帧及 UDP v3 地址发现。真实 GameHub 客户端已进入存储、商城及时间初始化阶段。新增可选本地服务模式，按请求顺序回复时间、用户/公共文件任务；未知任务返回明确失败。用户文件持久化通过重启读回和身份隔离测试。

M0、认证通道、时间和最小公共配置已有动态证据；第一阶段整体未完成。公共配置失败已解除，实际 the.json 上传与重登读回通过。80/230 基础玩家数据已使界面进入新玩家引导，仍有“服务器繁忙”提示；背包与完整大厅未验证。公共档案保存通过测试及真实历史报文校验，实际 UI 上传—重登闭环待完成。运行的是本地测试身份，不是腾讯账号认证。全部动态测试使用独立客户端克隆和 Wine prefix，临时 CA 已恢复；原件保持不变。用户已要求及时 Git commit。

用户先暂停并提供 AlphaBuild，完成审阅后再次要求继续。当前已据此实现票据，并独立恢复了目标版本新增的协议 210；参考目录仍保持原样。

下一次工作从这些步骤接续：

1. 阅读 PUBLISHER_FILES.md，复用 client-work/CODOL、wine10-prefix 和 server-data；两个公共文件已生成，无需再次覆盖。启动 Rust AUTH+LSG+STUN 及 --local-services。
2. 完成 8/3 公共档案的真实上传—重登验证，并定位 80/173、93/1 与引导界面的忙碌提示。不要重新调查已经确认的配置密钥或回执队列。
3. 目标大厅使用协议 210：HKDF-SHA1、AES-128-CBC、HMAC-SHA1。不要照搬 AlphaBuild 的旧大厅 3DES 帧；3DES 目前用于 AUTH 票据。
4. 80/70 更像商城目录，仍待恢复；已确定 80/230 的玩家数据结果结构，当前只有空初始账号记录，没有起始物品。继续结合目标资源恢复角色、武器及权益。
5. 文件和公共档案均按 AUTH 的 title/UID 隔离；引入完整玩家业务模型及 SQLite 后，以大厅→装备修改→重登→保留状态验收。当前完整检查为 39 项 Rust、5 项 Python 测试通过。每个验证阶段及时提交代码和接续文档。

研究原件在仓库的上一级：`../CODOL/`、`../CODO_OfflineLauncher.exe`、`../Installer/`。362 个 `.ifs` 容器已核对文件名、大小、nifs 文件头并记录 SHA-256；容器内部块和资源完整性仍未知。没有可用的本地官方成功抓包，实验依赖客户端解析路径及可观察的状态变化。

工具提示：shell 未找到 `rg`，使用 `find`。Homebrew Rust/LLVM 存在链接错误，使用 sh tools/cargo.sh 选择 Rust 1.96.0。静态工具在 .venv。Ghidra 12.1.4 已解压到 .tools/ghidra，并编译了 Mac arm64 decompiler；已有目标函数结果在 research-output/ghidra/auth-targets.c，使用 tools/ghidra/DecompileTargets.java 导出。GameHub 10 已验证、11 未做客户端实验。OpenSSL 使用 /opt/homebrew/bin/openssl。Parallels Windows 11 Arm 已准备好。

后续每次研究更新 CLIENT_BASELINE.md 的证据、方法、地址类型和置信度；需求或技术路线改变时更新方案，并在此处更新已完成状态及下一步。先前的“双客户端优先”已被最新的“登录、背包、大厅优先”替代。
