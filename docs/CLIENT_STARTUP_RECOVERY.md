# 2026-10-07 客户端副本启动恢复

结论：本轮 Wine/IIPS 启动阻塞已排除。卡住的是研究副本中的 `IIPS/IIPSDownload/ff_init_common_gfx_V43.ifs` 文件打开；将该文件保留在忽略目录，用原始样本中哈希一致的内容重新写入新文件后，客户端恢复 AUTH、大厅和地图连接。未改原始客户端，也没有清空账号、库存或钱包。

## 证据与处理

- 独立克隆原有 Wine prefix 为 `client-work/wine10-diagnostic-20261007`，仍在同一个 IIPS 阶段阻塞。这排除了“仅换 prefix 即可恢复”的假设。
- `client-1791309359354` 开启 `--file-trace`，末尾为上述文件的 SetFileAttributesW / NtCreateFile。普通 Python `os.open(O_RDONLY|O_NONBLOCK)` 也阻塞，说明故障已经在普通文件打开层面，不是 Rust 游戏连接消息导致。
- 原始样本中的同名文件可正常读取，大小 26504032，SHA-256 `a4bac3f12ef090773ee5fc5017e5f88dae600f2d81005e0f67257c02b78ff667`，匹配 `fixtures/resources-3.3.3.3.json`。
- 故障 inode 6280493 通过 rename 保留为 `research-output/blocked-resource-20261007/ff_init_common_gfx_V43.ifs.blocked`；未删除或覆盖。使用字节复制写入研究副本的新 inode，不继承扩展属性，再验证完整 SHA-256。记录为同目录 `repair.json`。
- 使用原有 prefix 的独立克隆 `client-work/wine10-recovered-20261007` 运行。`client-1791309533210` 恢复完整初始化并发出真实 connect；后续多次重新启动均正常。

这是“故障文件对象替换后恢复”的可重复证据，尚未证明底层 APFS/Wine/macOS 阻塞的确切成因。旧 Wine server 中有被终止后仍处于内核退出状态的进程；不要把它们误认为仍有可用实验服务，也不要再次读取保留的故障文件去制造新的阻塞进程。

## 工具改动与接续运行

`tools/gamehub_run.py --prefix-name NAME` 支持选择 `client-work` 下具备同一 engine marker 的独立实验 prefix。名称只允许简单字母、数字、下划线和连字符，不自动覆盖或重置已有容器。当前正常容器名为 `wine10-recovered-20261007`，认证和业务数据仍来自原来的 `client-work/server-data`。

```sh
sh tools/cargo.sh build --workspace --locked
python3 tools/gamehub_run.py client --prefix-name wine10-recovered-20261007 \
  --seconds 900 --title-id-hex 3ea --rust-auth --local-auth-tickets \
  --rust-lobby --rust-stun --rust-game --local-services --trust-lab-cert --quiet-network
```

仅文件打开排查时使用 `--file-trace`；它会较快触发 64 MiB 日志上限，`client-1791309533210` 就是因此被 harness 正常结束。平常使用上面的无 file-trace 命令。

Harness 现在捕获 SIGTERM，使其同 SIGINT 一样进入 finally，停止自己启动的进程并恢复研究副本的临时证书。清理期间忽略第二个 SIGINT/SIGTERM，防止在恢复中途被打断；SIGKILL 无法触发清理。实际 SIGTERM 验证已确认 `finished_unix` 和 `lab_certificate_restored=true`。

Python 13 项测试通过；prefix 克隆/修复实物都在忽略目录，不加入 Git。


## 同日第二次恢复

`client-1791354527177` 旧prefix卡在Wine loader IPC；独立prefix的 `client-1791354730132` 文件跟踪再次停在同一个IFS的SetFileAttributes/NtCreateFile。采样见 `research-output/wine-startup-91762.txt` 与 `wine-client-startup-92872.txt`。再次将故障inode6701441保留到 `research-output/blocked-resource-1791354904/`，从原始样本写入新inode7210064，校验SHA-256仍为上文的a4bac3f1…ff667；修复记录为该目录repair.json。不要重新打开保留的故障对象。

当前运行prefix改为 `wine10-play-20261007`。`client-1791355008373` 已恢复认证与游戏连接。只读探针编译使用另一个prefix `wine10-probe-build-20261007`，避免与运行中的游戏争用构建环境。旧prefix和原始客户端均保留。复发进一步确认替换文件对象有效，底层文件系统阻塞原因仍未确定。
