# Ubuntu 远端双客户端验收

本机 macOS ARM64 交叉编译 Linux x86_64 release，Rust 1.96.0、Zig 0.15.2、glibc 目标 2.35，使用 Ubuntu Jammy OpenSSL 3 头文件和库。产物在 Ubuntu 22.04.4 LTS 运行，上传前后 SHA-256 一致。后续构建均在本机进行；云主机只运行成品和联调。

测试主机为脱敏的 2 核、1963 MiB Ubuntu 环境，使用独立目录与专用账号。现有业务监听未修改，也未新增公网放行端口。AUTH／大厅经 SSH TCP 转发，地址发现与游戏 UDP 经 SSH 数据帧转发；因此本结果验证远端 Ubuntu 程序和跨主机联调，不代表公网 UDP 直连或真实丢包条件验收。

| 项目 | 结果 |
| --- | --- |
| Linux 成品运行 | ELF x86_64、系统 OpenSSL 3 和 glibc 依赖满足，地图及补齐 4 人配置装配成功 |
| 地址发现 | 20 次请求全部应答；中位 RTT 27.81 ms，备用端口应答与客户端源端口保持验证通过 |
| 双端核心联机 | 两个原生客户端完成认证、开房、浏览、加入、Bot 让位、准备和同图进入 |
| 真人交火与重生 | 普通鼠标开火产生 A 1 击杀／100 分、B 1 死亡；双方记录一致，B 重生血量 100 |
| 真人移动同步 | B 移动后远端角色与本地位置误差约 0.66 单位，四名角色模型与动画树均加载 |
| 共同结算 | 四行结果一致，首局比分 900:1000、时长 129 秒 |
| Tab 延迟 | 真人行读取实际快照 ACK RTT，原生截图显示约 58～68 ms，随采样更新；Bot 为 0 |
| 结算后再开局 | 同一 Ubuntu 进程和原生房间恢复 2 真人＋2 Bot，再次进入状态 10，新时钟起算 |
| 资源占用 | 测试进程 RSS 约 61 MiB、记录的峰值约 74 MiB |

Tab 使用通过 native 序号／ACK 校验的游戏消息测量往返时间，EWMA 平滑后最多每秒发布一次。尚无样本的真人为 -1，不使用默认 0 假定本地延迟；Bot 不经过网络，保留 0。该值包含游戏数据通道、调度和客户端处理时间，不是 ICMP ping。

共享世界结束时仅释放房间的进行中约束，保留旧连接发送结算；下一次分配产生新 ID。房主完整名单明确移除旧 Bot 后，服务端重新进行原生 Bot 加入与队伍接纳，避免结算后补位被旧分配锁住。

本机修复验证：455 项服务端测试通过、28 项资源依赖测试忽略，严格 Clippy 通过。原生探针仅做只读检查；输入使用普通游戏鼠标和键盘，EXE／DLL／资源内容未改写。

合入现有主线的独立版本另通过 468 项服务端测试、29 项忽略及严格 Clippy；结果见 `final-integration-tests.log` 和 `final-integration-clippy.log`。原生证据使用上述冻结的本机交叉编译产物，不替代其它后续主线功能的原生回归。

证据位于忽略目录 `research-output/remote-multiplayer-acceptance-20261015/`：`remote-udp-probe.json`、`linux-binary-repaired.json`、`local-repair-tests.log`、`local-repair-clippy.log`，以及两轮 native 目录内的 `remote-proof.json`、`remote-pvp-proof.json`、`tab-ping-proof.json` 和 `reopen-proof.json`。

测试期间在云主机构建阶段遇到 SSH 无响应，后续改为本机交叉编译，根因未经系统级诊断确认。原生加载／结束边界有被严格解码拒绝的过渡输入和一个未支持的参与者消息，不影响上述已核验流程；完整模式、装备、公网 UDP 直连仍需分别验收。测试游戏、隧道和自建服务已关闭，两套临时客户端的连接配置和证书恢复到摘要一致的隔离基线，未操作日常客户端。中断后遗留配置的私有副本与恢复摘要保存在 `cleanup-proof.json` 和 `configuration-recovery/`。
