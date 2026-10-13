# 原生客户端启动和会话隔离

本机实际客户端为`server/client-work/CODOL`，工作区`CODOL`是兼容符号链接。关键EXE、版本、IIPS DLL及列表文件已按fixture核对完整SHA，原目录中不同的设置、日志和唯一`_co.tlg`保存在`research-output/client-single-copy/original-settings/`。没有完成全部资源文件的两份逐字节对照，不能把关键身份校验扩大为全目录内容一致证明。账号、库存和钱包仍在独立的`client-work/server-data`。

当前Wine10原生启动已在系统重启后恢复。`wine10-throwables-20261011`的`client-1791886330960`通过AUTH、大厅、6772条库存、阿富汗地图和熔岩ADS画面对照；诊断已正常结束且两处临时证书恢复。当前没有活动正常会话，旧`map-experiment.txt`仍指向已结束实验，不能直接据它暂停或恢复游戏。Wine11的完整对局路径仍未验收。

重启前，原生ARM64 Python的普通打开与Wine server的`open_fd`都卡在系统文件调用。当前资源`ff_init_common_gfx_V43.ifs`通过`O_EVTONLY`可打开并查询到无记录锁、无租约，实际读取仍阻塞；这些区别记录在`startup-root-cause/event-open-result.json`和`read-modes-result.json`。系统盘由约3.4GiB清至48GiB、工作区由约3.4GiB清至6.6GiB后，原有等待仍未解除。

重启后，相同inode13261206的26504032字节资源在约15毫秒内完成普通读取，完整SHA保持`a4bac3f1…ff667`；没有替换资源文件。原有退出／U状态进程消失，游戏随后恢复。`lava-ads-after-reboot/resource-read-check.json`和`acceptance-proof.json`提供读取、实际对局及证书清理证明。证据支持本次直接阻塞位于系统文件访问／退出状态；尚无内核栈证明具体APFS内部成因，也不能仅凭重启恢复断言文件损坏、硬盘故障或某个Wine版本缺陷。

重启前的停止证据：91个Wine及Windows子进程收到SIGTERM／SIGKILL，其中51个退出，40个仍由`ps`显示为`?Es`且RSS为0。macOS本机`ps(1)`将E解释为正在退出；这些进程在强制结束后仍残留，`proc_pidinfo`对其返回ESRCH，不能把它们当作可用游戏或服务实例。停止结果及进程记录在`lava-ads-after-wine-cleanup/wine-stop-result.json`和`remaining-kernel-processes.json`，本次系统重启后残留已消失。

停止及容器清理后，干净的`wine11-lava-20261013`环境在`client-1791871854997`中启动真实游戏PID，但35秒内没有可见窗口，未运行状态探针或截图。该诊断已经结束，manifest确认临时证书恢复，正常服务资料全量哈希不变；熔岩ADS画面没有验收通过。证据在`lava-ads-after-wine-cleanup/result.json`和`diagnostic-cleanup.json`，本次release构建通过。

工作区已删除21个结束的旧Wine容器，保留7个环境：`wine10-prefix`、`wine10-loadout-build-20261007`、`wine10-probe-roster-20261010`、`wine10-throwables-20261011`、`wine10-lava-20261013`、`wine11-lava-20261013`和`wine11-weapon-native-20261013`。小体积注册表／标记记录保存在`lava-ads-after-wine-cleanup/removed-prefix-metadata/`，实际客户端、账号服务资料和原生采集证据保留；旧容器的历史manifest不能当作仍存在的可运行prefix。清理实际释放806006784字节，见`prefix-cleanup.json`，逻辑目录总大小与APFS实际回收空间不同。

干净的`wine10-lava-20261013`环境在`client-1791868486206`中启动真实游戏进程，但没有可见窗口，只读探针30秒超时，未到大厅。OS采样见`lava-sight-native-check/client-startup.txt`，游戏线程的栈集中在Wine syscall dispatcher；这不能直接归因于IFS，也不证明其底层成因。该诊断已结束，manifest确认临时证书恢复，正常服务资料全量哈希保持一致，新增截图数0。其服务端使用`0eb7fe5`的已构建release基线，不作为其它正在修改功能的验收。

`gamehub_run.py`对全部Wine环境使用客户端目录中的共享独占锁，先于配置、证书和本地服务变更获取；同时识别尚未使用该锁的原有项目harness或精确游戏路径。第二个会话明确退出，不关闭已运行会话。锁持续到本次停止进程、恢复证书并写入结束manifest，进程退出后由操作系统释放。Wine环境自身的锁继续限制同一prefix内的管理操作。

`research-output/lava-sight-native-check/shared-client-lock-proof.json`通过真实第二次启动确认跨prefix拒绝、退出码2，以及dwauth/stun/iips配置和两处证书的SHA全部不变。Python回归覆盖跨prefix争用、进程退出释放、旧harness识别和失败获取后的释放。

文件对象阻塞与Wine server失联需要分别诊断。`IIPS/IIPSDownload/ff_init_common_gfx_V43.ifs`曾卡在普通文件打开层，保留故障对象、用哈希一致内容写入新对象后恢复AUTH、大厅和地图连接。以下为该处理的实物依据；当前仅保留一份客户端，不能再假定有另一份完整原件可供重写。

## 证据与处理

- 独立克隆原有 Wine prefix 为 `client-work/wine10-diagnostic-20261007`，仍在同一个 IIPS 阶段阻塞。这排除了“仅换 prefix 即可恢复”的假设。
- `client-1791309359354` 开启 `--file-trace`，末尾为上述文件的 SetFileAttributesW / NtCreateFile。普通 Python `os.open(O_RDONLY|O_NONBLOCK)` 也阻塞，说明故障已经在普通文件打开层面，不是 Rust 游戏连接消息导致。
- 原始样本中的同名文件可正常读取，大小 26504032，SHA-256 `a4bac3f12ef090773ee5fc5017e5f88dae600f2d81005e0f67257c02b78ff667`，匹配 `fixtures/resources-3.3.3.3.json`。
- 故障 inode 6280493 通过 rename 保留为 `research-output/blocked-resource-20261007/ff_init_common_gfx_V43.ifs.blocked`；未删除或覆盖。使用字节复制写入研究副本的新 inode，不继承扩展属性，再验证完整 SHA-256。记录为同目录 `repair.json`。
- 使用原有 prefix 的独立克隆 `client-work/wine10-recovered-20261007` 运行。`client-1791309533210` 恢复完整初始化并发出真实 connect；后续多次重新启动均正常。

这是“故障文件对象替换后恢复”的可重复证据，尚未证明底层 APFS/Wine/macOS 阻塞的确切成因。旧 Wine server 中有被终止后仍处于内核退出状态的进程；不要把它们误认为仍有可用实验服务，也不要再次读取保留的故障文件去制造新的阻塞进程。

## 运行与清理

`tools/gamehub_run.py --prefix-name NAME`支持选择`client-work`下具备同一engine marker的独立实验prefix。名称只允许简单字母、数字、下划线和连字符，不自动覆盖或重置已有容器。正常业务数据仍来自`client-work/server-data`，恢复正常会话前须确认实际进程、共享锁、精确manifest和原有地图catalog。已结束manifest不能用于暂停活动会话，也不能把包装启动器PID当作真实游戏PID。

仅文件打开排查时使用`--file-trace`；它会较快触发64MiB日志上限，`client-1791309533210`因此由harness正常结束。正常游玩命令与配置见[LOCAL_PLAY.md](LOCAL_PLAY.md)。代码检查不能替代原生运行，当前恢复依据为重启后的实际AUTH、大厅、地图及ADS验收。

Harness捕获SIGTERM，使其同SIGINT一样进入finally，停止自己启动的进程并恢复客户端的临时证书。清理期间忽略第二个SIGINT/SIGTERM，防止在恢复中途被打断；SIGKILL无法触发证书清理。实际SIGTERM验证确认`finished_unix`和`lab_certificate_restored=true`。只读探针编译使用独立的项目prefix，避免与运行游戏争用构建环境。

Wine11可能以`start.exe`作为启动器并将真实游戏进程交给PPID1。工具从精确客户端路径和本prefix环境或映射文件确认归属，manifest分别保存真实`pid`、`launcher_pid`和`harness_pid`。启动器成功退出时继续在原90秒期限内等待已验证的真实游戏；非零退出立即报告实际状态码，没有真实游戏则明确超时。回归覆盖启动器先退出、游戏稍后出现和真正失败；这项进程识别不等于Wine11完整渲染、输入和业务流程已经验收。

`client-1791868764186`的文件跟踪停在公共资源`ff_init_common_gfx_V43.ifs`的SetFileAttributes/NtCreateFile。重启前，该文件的APFS克隆调用和`startup-preserved-1791586547`保留样本的普通读取均未完成，进程处于内核U状态；未写入替换资源，未重新打开已知`.blocked`对象。受阻读取、未完成克隆及其进程标识记录在`lava-sight-native-check/resource-read-blockage.json`。当前在相同资源对象上恢复正常启动，具体底层根因仍未确定。

## 文件对象替换的重复证据

`client-1791354527177` 旧prefix卡在Wine loader IPC；独立prefix的 `client-1791354730132` 文件跟踪再次停在同一个IFS的SetFileAttributes/NtCreateFile。采样见 `research-output/wine-startup-91762.txt` 与 `wine-client-startup-92872.txt`。再次将故障inode6701441保留到 `research-output/blocked-resource-1791354904/`，从原始样本写入新inode7210064，校验SHA-256仍为上文的a4bac3f1…ff667；修复记录为该目录repair.json。不要重新打开保留的故障对象。

`wine10-play-20261007`的`client-1791355008373`取得恢复后的认证与游戏连接证据。复发进一步确认替换文件对象有效，底层文件系统阻塞原因仍未确定。故障实物与prefix保留在忽略目录，不加入Git；被保留的故障对象不应再次打开。
