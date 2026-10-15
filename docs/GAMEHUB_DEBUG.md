# GameHub 客户端运行与 AUTH 诊断

GameHub Wine/Proton 的项目独立环境已用于固定3.3.3.3客户端的认证、大厅、地图与对局。服务端使用Rust；诊断进程、客户端根目录、可写资料和prefix按实验归属隔离。

## 启动

正常游玩使用[本地入口](LOCAL_PLAY.md)，诊断可使用：

```sh
sh tools/cargo.sh build --locked --workspace
python3 tools/gamehub_run.py client --prefix-name GAME_PREFIX \
  --seconds 900 --title-id-hex 3ea \
  --rust-auth --local-auth-tickets --rust-lobby --rust-stun --rust-game \
  --game-state GAME_STATE_JSON --local-services --trust-lab-cert --quiet-network
```

`--seconds` 是诊断期限，不是正常游玩会话限制。`EXPERIMENT_JSON`取本次输出的manifest；服务就绪、进程和配置恢复见[启动边界](BACKEND_STARTUP.md)。同一客户端根目录只允许一个受管会话，不能通过换prefix绕过根目录锁。

## AUTH 观察

本版客户端使用HTTPS `POST /auth/` 和双层JSON。title ID为1002（十六进制3ea），字段按实际请求校验；不能只根据配置字符串判断已完成认证。TLS ClientHello、JSON请求、票据接受和大厅握手分别有独立观察点，协议见[AUTH](AUTH_LOBBY_PROGRESS.md)。

CA文件使用相对路径，客户端根目录与IIPS工作目录都可能成为读取位置。启动器在受控副本中暂存实验信任配置，退出恢复；ClientHello前EOF须先核对CA和实际工作目录，不能直接归因为票据失败。

## 只读状态与资源

```sh
python3 tools/client_state.py --build --prefix-name PROBE_PREFIX
python3 tools/client_state.py --experiment EXPERIMENT_JSON
python3 tools/client_state.py --experiment EXPERIMENT_JSON --net-constants
```

编译在对应prefix客户端停止时执行，采样在实验运行时执行。探针校验样本和PID，不写内存、不调用游戏函数、不读取密钥。资源loaded、NCS初始化、连接状态和快照ACK分别判断；完整方法见[只读诊断](ONBOARDING_PROGRESS.md)。

文件访问停滞、无窗口及不可中断Wine状态须按[客户端恢复](CLIENT_STARTUP_RECOVERY.md)处理；重复启动失效环境不能产生有效协议结论。原始日志、证书、私有抓包和账号材料留在忽略目录。
