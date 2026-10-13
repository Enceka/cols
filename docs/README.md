# 接续入口

TDM出生与重生按地图点位和阵营使用原生朝向，23图准备、预测角度和实机范围见[出生朝向](SPAWN_ORIENTATION.md)。

高楼／潜艇基地小地图底图比例的修正、原生投影读数及定向画面见[小地图投影](MINIMAP_PROJECTION_PROGRESS.md)。

连杀奖励的本版资源、背包布局、原生HUD／动作槽及服务端边界见[连杀接入准备](KILLSTREAK_PREPARATION.md)。当前具备权威进度、三项背包预览和特殊25杀核弹实现；核弹使用额外奖励槽1，王牌槽5保持独立。普通奖励效果尚未接入；核弹自然25杀、右侧图标、按7调用、倒计时、FX创建及结算已在最新版Rust启动器下验收，完整vision／慢动作仍待接入。

Bot普通跳跃、助跑、路线随机偏好与脱困的实现和后台地图回归见 [Bot导航提升](BOT_NAVIGATION_PROGRESS.md)；原生翻越、梯子及专用跳跃动画仍待接入。

原生开局、角色喊话与胜负播报的接入准备见 [原生语音](VOICE_PROGRESS.md)：模式介绍→开战指令、男女换弹、手雷／闪光喊话和胜局播报已有安全屋／坠机原生验收；正常23份地图准备已启用语音，保留原账号和7500分／10分钟。

2026-10-10接续：[原生选图与结算](MAP_SELECTION_PROGRESS.md)、[新参考文档功能对齐](REFERENCE_ALIGNMENT.md)。安全屋原生箭头、12人Bot入房、昵称、胜负结算、阵营同步和离房重建已通过实机验证。用户要求自主继续、尽量少截图，Bot使用不区分阵营的`Bot n`名称。坠机之地独立资源、TDM及小地图底图已验证；首次出生贴地和快速切枪已修复；继续纯Rust及3.3.3.3协议，不照搬2.6.9.7地址。

本仓库诊断客户端已停止。常规prefix为`wine10-throwables-20261011`，只读探针构建prefix为`wine10-probe-roster-20261010`；普通账号数据仍在`client-work/server-data`，新地图采集用独立服务目录。其余武器待验收项见[原生武器行为接续](WEAPON_NATIVE_PROGRESS.md)。下文为2026-10-07历史阶段记录，不能覆盖上述最新状态。

最后更新：2026-10-07。本仓库用于 CODOL 3.3.3.3 的 Rust 服务端重建。

- [实施方案](REVERSE_ENGINEERING_PLAN.md)：范围、技术路线、Mac 运行策略、模块、里程碑及验收标准。
- [已发现的事实](CLIENT_BASELINE.md)：样本哈希、文件偏移、配置、资源清单、环境与未验证假设。
- [启动链与配置读取](BOOTSTRAP_FINDINGS.md)：CIL 与原生指令的静态结论、RVA/VA 和下一步交叉引用。
- [工程基础与运行手册](FOUNDATION.md)：已经实现的能力、构建/测试/观测命令、输出格式与限制。
- [GameHub 动态调试](GAMEHUB_DEBUG.md)：真实 ClientHello、HTTPS AUTH 请求、title_id 对照实验、运行工具和响应逆向入口。
- [AlphaBuild 审阅](ALPHA_BUILD_REVIEW.md)：用户提供的 1.1.1.13 实现、与 3.3.3.3 的交叉核对和移植依据。
- [AlphaBuild逻辑移植路线](ALPHA_RUST_PORT_PLAN.md)：用户确认纯Rust，按完整玩法模块移植并适配3.3差异。
- [2.6.9.7加载文档对照](LOADING_2697_REVIEW.md)：本版IFS明文清单、M4跨包资源与模型表装配函数的核对。
- [玩家背包配装](PLAYER_LOADOUT_PROGRESS.md)：按账号读取保存的背包，合并默认件与玩家配件，校验所有权和组合参数。
- [持枪模型与动画绑定](VIEWMODEL_PROGRESS.md)：实际DObj/XAnim证据、手臂模型单变量对照及默认资源修正。
- [AUTH 与大厅通道](AUTH_LOBBY_PROGRESS.md)：Rust 认证票据与协议 210 的真实客户端验证、UDP 地址发现、业务帧解密。
- [业务初始化与文件服务](SERVICE_INITIALIZATION.md)：任务 FIFO、时间、存储格式与早期公共配置失败。
- [公共配置与玩家初始化](PUBLISHER_FILES.md)：配置解密与解析、80/230 和档案格式。
- [新玩家引导与只读调试](ONBOARDING_PROGRESS.md)：真实公共档案上传/重登、初始化与只读状态探针。
- [私有档案与初始库存](PROFILE_INVENTORY_PROGRESS.md)：目标 233/19 定义、8 件库存和仓库界面。

用户最新要求：先恢复登录、背包和大厅，然后推进本地房间/人机与双客户端最小对局。优先使用 GameHub Wine/Proton 在 Mac 上运行，Parallels Windows 保留作对照；服务端使用 Rust。

当前入口：[背包与全物品](PLAYER_LOADOUT_PROGRESS.md)、[移动与音频](GAMEPLAY_INPUT_PROGRESS.md)。通用资源配装、主副武器和CS6800注册已接通，用户确认切枪恢复；6772项物品已授予。相机旋转参考、落地推进和胶囊碰撞已补修，实机静止位置一致，移动误差中位约0.094单位；特殊地形仍待核验，伤害/命中/AI未实现。

当前游戏prefix为 `wine10-loadout-20261007`；探针编译用 `wine10-loadout-build-20261007`。音频32位COM缺失已经修复注册，后续需听音确认。运行实验仍有900秒期限，退出状态以experiment.json为准。

最近完整检查通过：157项Rust、30项Python（含8项回放结构测试，另1项真实回放诊断显式ignored）及fmt/Clippy。曾有UDP测试冷构建后2秒等待超时、同一binary重跑成功；没有盲目延长超时。普通测试不代替真实方向、移动与音频验收。

下一次工作：

1. 默认手臂已通过单变量对照改为viewhands_tf141，实际DObj/XAnim已读到。默认配件已补齐，当前背包配装见新文档；idle0x400→0x401实验没有修复姿态，不要重复。详见持枪模型文档。
2. ADS连续状态已接入；扩展背包组合时先补充实测参数，再接入地图碰撞与权威位移。每帧固定出生坐标不是移动实现；不把客户端预测动作当作服务器完成。
3. 复用已验证native方向、NCS CRC、时钟配置和ACK输入base。改PS字段时核对目标版本的235项字段表，不能照搬Alpha布局。
4. 复用现有客户端与业务数据，不重新初始化账号。商城切换暂缓；剧情首关仍缺无21/1路径的security key。
5. 每个可验证阶段Git commit；没有要求推送。原始抓包、资源、账号和密钥保留在忽略目录。

研究原件在仓库的上一级：`../CODOL/`、`../CODO_OfflineLauncher.exe`、`../Installer/`。362 个 `.ifs` 容器已核对文件名、大小、nifs 文件头并记录 SHA-256；容器内部块和资源完整性仍未知。没有可用的本地官方成功抓包，实验依赖客户端解析路径及可观察的状态变化。

工具提示：shell 未找到 `rg`，使用 `find`。Homebrew Rust/LLVM 存在链接错误，使用 sh tools/cargo.sh 选择 Rust 1.96.0。静态工具在 .venv。Ghidra 12.1.4 已解压到 .tools/ghidra，并编译了 Mac arm64 decompiler；已有目标函数结果在 research-output/ghidra/auth-targets.c，使用 tools/ghidra/DecompileTargets.java 导出。GameHub 10 已验证、11 未做客户端实验。OpenSSL 使用 /opt/homebrew/bin/openssl。Parallels Windows 11 Arm 已准备好。

后续每次研究更新 CLIENT_BASELINE.md 的证据、方法、地址类型和置信度；需求或技术路线改变时更新方案，并在此处更新已完成状态及下一步。先前的“双客户端优先”已被最新的“登录、背包、大厅优先”替代。

闪光／震撼、原生shellshock及参考脚本抗性边界见 [TACTICAL_STATUS_PROGRESS.md](TACTICAL_STATUS_PROGRESS.md)。
