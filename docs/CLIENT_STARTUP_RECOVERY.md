# 原生客户端启动和会话隔离

本机实际客户端为`server/client-work/CODOL`，工作区`CODOL`是兼容符号链接。关键EXE、版本、IIPS DLL及列表文件已按fixture核对完整SHA，原目录中不同的设置、日志和唯一`_co.tlg`保存在`research-output/client-single-copy/original-settings/`。没有完成全部资源文件的两份逐字节对照，不能把关键身份校验扩大为全目录内容一致证明。账号、库存和钱包仍在独立的`client-work/server-data`。

当前完整原生启动仍待恢复；旧`wine10-throwables-20261011`环境报告Wine server失联，Wine11已取得真实游戏进程但尚无完整修复后对局验收。旧`map-experiment.txt`指向已结束实验，不能直接当作活动正常会话。已有AUTH、大厅和地图成功证据不代表这些环境当前均能再次启动。

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

仅文件打开排查时使用`--file-trace`；它会较快触发64MiB日志上限，`client-1791309533210`因此由harness正常结束。正常游玩命令与配置见[LOCAL_PLAY.md](LOCAL_PLAY.md)，当前启动障碍未因离线检查通过而解除。

Harness捕获SIGTERM，使其同SIGINT一样进入finally，停止自己启动的进程并恢复客户端的临时证书。清理期间忽略第二个SIGINT/SIGTERM，防止在恢复中途被打断；SIGKILL无法触发证书清理。实际SIGTERM验证确认`finished_unix`和`lab_certificate_restored=true`。只读探针编译使用独立的项目prefix，避免与运行游戏争用构建环境。

Wine11可能以`start.exe`作为启动器并将真实游戏进程交给PPID1。工具从精确客户端路径和本prefix环境或映射文件确认归属，manifest分别保存真实`pid`、`launcher_pid`和`harness_pid`。这项进程识别不等于Wine11完整渲染、输入和业务流程已经验收。

## 文件对象替换的重复证据

`client-1791354527177` 旧prefix卡在Wine loader IPC；独立prefix的 `client-1791354730132` 文件跟踪再次停在同一个IFS的SetFileAttributes/NtCreateFile。采样见 `research-output/wine-startup-91762.txt` 与 `wine-client-startup-92872.txt`。再次将故障inode6701441保留到 `research-output/blocked-resource-1791354904/`，从原始样本写入新inode7210064，校验SHA-256仍为上文的a4bac3f1…ff667；修复记录为该目录repair.json。不要重新打开保留的故障对象。

`wine10-play-20261007`的`client-1791355008373`取得恢复后的认证与游戏连接证据。复发进一步确认替换文件对象有效，底层文件系统阻塞原因仍未确定。故障实物与prefix保留在忽略目录，不加入Git；被保留的故障对象不应再次打开。
