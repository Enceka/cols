# 原生客户端启动、隔离与恢复

原生客户端在本机独立副本和 GameHub Wine/Proton 环境中运行。EXE、版本、IIPS DLL及清单按fixture校验；关键文件一致不能扩大为全目录、全部资源完整性证明。账号、库存与钱包保存在独立服务资料目录，完整原始资源和故障材料留在忽略目录。

## 会话归属与独占锁

`gamehub_run.py` 对同一客户端根目录的全部prefix使用共享独占锁，先获取锁，再修改配置、安装临时信任或启动后端。第二个会话明确退出，不替使用者关闭已有会话；锁覆盖进程清理、配置／证书恢复和结束manifest写入，由操作系统在进程退出时释放。

同时识别尚未使用该锁的项目harness和精确游戏路径。残留锁文件不代表有活动进程；已结束manifest不能用于暂停／恢复新会话。manifest区分真实游戏 `pid`、`launcher_pid` 与 `harness_pid`，不能把启动器PID当作游戏PID。

`--prefix-name` 只选择有正确engine marker的项目环境，不自动重置已有prefix。不同真人客户端需独立可写根目录及prefix，见[多人](MULTICLIENT_ACCEPTANCE.md)。

## 文件访问阻塞

已观察到公共资源 `ff_init_common_gfx_V43.ifs` 的 `SetFileAttributesW`／`NtCreateFile`／普通系统open停滞。该资源基线长度26504032，SHA-256 `a4bac3f12ef090773ee5fc5017e5f88dae600f2d81005e0f67257c02b78ff667`。仅换prefix不能保证恢复。

有完整哈希一致来源时，保留故障文件对象，以独立文件对象重建研究副本并核对长度／完整SHA，可恢复启动；系统恢复后相同对象也有重新可读的证据。确切APFS／Wine／内核根因未确定，不能据此断言资源损坏或硬件故障。不要再次读取已知阻塞对象制造更多不可中断进程。

`Us` 或退出中的残留进程不能当作可用游戏。没有窗口、无UDP或探针超时不证明协议错误；先核对后端就绪、进程归属、文件调用和前台。`--file-trace` 仅用于文件诊断，日志较快达到预算，正常游玩见[入口](LOCAL_PLAY.md)。

## 清理与恢复

SIGINT／SIGTERM进入finally，停止自有进程并恢复临时INI／CA；清理时避免重复信号中断，SIGKILL不能执行恢复。核对manifest的结束记录和 `lab_certificate_restored=true`，异常中断使用启动器恢复入口，不能只因进程消失就假定恢复完成。

Wine可将实际游戏交给其它父进程；工具按精确客户端路径和prefix环境／映射校验归属。启动器成功退出时仍在有界期限内等待真实游戏，非零退出报告实际错误；该识别能力不等于Wine11完整渲染和对局已验收。

## 验证范围

跨prefix争用拒绝、锁释放、旧harness识别、真实游戏发现和SIGTERM恢复均有工具回归及独立验证。Wine10原生AUTH、大厅、库存、地图和ADS已成功运行；Wine11完整对局、小时级稳定性及全部故障恢复仍需独立验收。服务就绪契约见[启动边界](BACKEND_STARTUP.md)。
