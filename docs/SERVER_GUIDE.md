# 独立服务器使用教程

独立服务器通过 `codol-server serve-auth` 运行认证、大厅、地址发现和权威游戏世界，无需启动游戏窗口或 Wine。多个客户端连接同一服务进程，按房间共享对局；退出一个客户端不会主动关闭整台服务。

当前 AUTH、大厅、STUN 和游戏 UDP 都只接受回环地址。可直接使用的是本机服务，以及通过专用转发链路接入的远端服务；把监听地址改为 `0.0.0.0` 会被代码拒绝。远端 Linux 双端核心流程已通过 SSH 转发联调，公网 UDP 直连尚未开放。

## 使用面板一键开服

在 `server/` 目录构建并启动管理面板：

```sh
sh tools/cargo.sh build --release --locked -p codol-server
target/release/codol-server admin
```

浏览器打开 `http://127.0.0.1:39080`，进入“开服设置”，选择服务器上的服务资料目录、默认地图和地图目录文件，设置端口与补位人数，然后点击“一键开服”。已有本机准备可使用自动填入的路径。

首次开服会自动生成配套证书／私钥和缺失的公共配置，并启动AUTH、大厅、STUN、游戏UDP与资源观察器；只有实际装配完成才显示“运行中”。已有玩家、公用文件及地图不会被重置。证书和设置保存在 `client-work/server-control/`，停止后保留，后续可直接再次开服。

“下载公开证书”提供客户端需要信任的证书；私钥只在服务器保存。证书过期或需更新时先停服，再使用“更新本地证书”，客户端相应更新信任。服务端就绪不自动完成客户端配置，接入方法见下文。

点击“停止服务”释放本次端口。关闭网页后服务器继续运行；退出管理进程时会回收其自有服务。Linux同样可运行此面板；管理地址仅监听回环，远端需要对应的SSH转发。

可用 `--state-dir /path/to/server-control` 指定独立持久化目录。详细权限、日志、配置及测试边界见[管理面板](ADMIN_PANEL.md)。下面的命令行方法用于手动管理与部署排查，不与面板同时占用同一组端口和资料目录。

## 准备服务程序与数据

本机从 `server/` 目录构建：

```sh
sh tools/cargo.sh build --release --locked -p codol-server
target/release/codol-server serve-auth --help
```

服务器运行时需要以下文件，不需要图形客户端进程：

| 内容 | 用途 |
| --- | --- |
| `codol-server` | 与部署平台匹配的可执行文件 |
| `server.pem`、`key.pem` | AUTH 的有效证书与配套私钥 |
| `service-data/` | 公共配置、物品规则及各个玩家的档案、库存、钱包 |
| 默认地图准备 | `--game-state` 指定的有效地图状态 |
| 地图目录及全部引用文件 | `--game-map-catalog` 按地图和模式选择资源 |
| 可写捕获目录 | 有界诊断记录及运行元数据 |

准备数据须来自固定 3.3.3.3 样本，地图名称存在不代表碰撞、武器、出生点和模型已准备。地图准备见[资源流程](MAP_RESOURCE_PREPARATION.md)，账号结构见[档案与库存](PROFILE_INVENTORY_PROGRESS.md)。

复制到另一台机器时，保持目录与引用关系，修正指向开发机的绝对路径。运行 `verify-maps` 检查部署副本中的引用和后台对局：

```sh
bin/codol-server verify-maps \
  --catalog maps/catalog.json \
  --output map-verification.json --seconds 60
```

此命令要求已准备完整地图，输出文件使用新名称。它验证服务端资源和模拟，实际客户端接入还需单独检查。

## 单独启动服务

在已有本机环境的 `server/` 目录执行：

```sh
target/release/codol-server serve-auth \
  --bind 127.0.0.1:39002 \
  --cert .tools/tls-lab/server.pem \
  --key .tools/tls-lab/key.pem \
  --local-title-id 1002 \
  --lobby-bind 127.0.0.1:39001 \
  --service-data-dir client-work/server-data \
  --stun-bind 127.0.0.1:3074 \
  --stun-alt 127.0.0.1:3078 \
  --game-bind 127.0.0.1:39004 \
  --game-state research-output/tdm-afghan-hud-state.json \
  --game-map-catalog research-output/native-tdm-map-catalog.json \
  --room-bot-fill 12 \
  --continuous-play \
  --game-capture-every 1000 \
  --capture-dir captures/dedicated
```

保持该终端运行。默认日志包含各端点就绪情况，地图加载完成后会出现 `experimental Rust game UDP endpoint ready`；只有 AUTH 监听成功，还不能开始游戏。

| 参数 | 实际作用 |
| --- | --- |
| `--local-title-id 1002` | 签发本版使用的本地身份票据；省略时不是完整登录服务 |
| `--service-data-dir` | 所有连接共用的服务资料根目录，按 title／UID 隔离账号 |
| `--game-state` | 默认入口的地图准备 |
| `--game-map-catalog` | 原生房间选图使用的地图／模式目录 |
| `--room-bot-fill 12` | 支持的自定义战术房间补到 12 人，真人加入时替换 Bot；允许 4～12 |
| `--continuous-play` | 持续处理游戏和已认证活动连接，不按诊断帧数主动结束 |
| `--game-capture-every 1000` | 降低普通游戏逐包采样频率，不降低模拟或快照频率 |

不需要自动补位时省略 `--room-bot-fill`。单真人加 11 Bot 的兼容模式使用 `--native-bot-rooms`，与补位选项互斥；冲锋 Bot 入口使用兼容模式及已准备的冲锋资源。

服务参数来自 `serve-auth` 命令行。`configs/local.toml` 是观察器配置，编辑它不会修改上述 AUTH、大厅和游戏端口。`admin` 提供本地诊断页面，也不代替游戏服务进程。

## 端口与资源观察器

| 本例地址 | 协议 | 用途 |
| --- | --- | --- |
| `127.0.0.1:39002` | TCP／TLS | AUTH |
| `127.0.0.1:39001` | TCP | 协议 210 大厅 |
| `127.0.0.1:3074`、`:3078` | UDP | 主／备用 STUN |
| `127.0.0.1:39004` | UDP | 游戏握手、输入和快照 |
| `127.0.0.1:39003` | TCP | 客户端 IIPS 资源请求观察器，另行启动 |

沿用本机启动器的 IIPS 连接配置时，新建 `configs/resources.local.toml`：

```toml
capture_dir = "../captures/resources"
max_connections = 16
max_sessions_per_run = 128
max_bytes_per_session = 1048576
max_chunks_per_session = 4096
idle_timeout_ms = 10000
session_timeout_ms = 30000

[[listeners]]
name = "resources"
bind = "127.0.0.1:39003"
```

在另一个终端运行：

```sh
target/release/codol-server observe --config configs/resources.local.toml
```

观察器只记录资源请求，不下载或分发游戏资源；客户端必须已有所需本地资源。不要同时运行默认 `observe` 配置，其 39001／39002 会与实际大厅和 AUTH 冲突。

## 客户端连接已运行的服务器

`native-launch` 和 `play.py` 都会自行启动后端，不能作为连接本例常驻服务的入口，否则会争用同一组端口。

当前没有集成“仅连接已有服务器并自动备份／恢复”的图形入口。已有独立客户端副本完成连接配置后，可以用启动器通用 `direct` 配置启动；这种方式不创建第二套后端，也不会自动调整或恢复客户端 INI／CA。

### 配置独立客户端副本

每个客户端使用独立可写目录、Wine prefix 和登录标识。保留修改前的连接文件及两处 CA 备份；以下内容只写入专用副本。

`dwauth.ini` 指向同一套大厅和 AUTH，末字段为 title ID：

```text
127.0.0.1 39001
127.0.0.1 39002
0x000003EA
```

`stun.ini` 为：

```text
127.0.0.1
```

`iips.ini` 与上面的资源观察器对应：

```ini
enable_iips=1
iips_url="http://127.0.0.1:39003/"
```

客户端根目录 `cacert.pem` 和 `IIPS/cacert.pem` 需要保留原根证书，并附加本次服务的公开证书 `server.pem`。私钥只放服务器；不能用替换整个根证书文件或关闭验证代替正确配置。

### 只启动客户端

在 `launcher/profiles/` 下创建私有配置 `dedicated-a.local.json`，将示例路径换成本机实际路径：

```json
{
  "version": 1,
  "client": {
    "executable": "/path/to/client-a/codoMP_client_shipRetail.exe",
    "sha256": "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
    "arguments": [
      "-q", "0", "-src=tgp", "-game_id", "29", "-zone_id", "83886082",
      "+set", "r_fullscreen", "0",
      "+set", "1011", "1", "+set", "3265", "12",
      "+set", "dw_autoStartGame", "0",
      "+set", "dw_username", "PlayerA"
    ],
    "runner": "/path/to/wine/bin/wine",
    "wine_prefix": "/path/to/prefix-a"
  },
  "routing": "direct",
  "routes": [],
  "state_dir": "../state/dedicated-a"
}
```

登录标识须对应服务器资料中的账号，限定 1～15 个 ASCII 字母、数字或下划线。`PlayerA` 只是占位值，随意更换会得到不同 UID，不会自动继承原账号库存。客户端昵称与登录标识分别处理；`-q` 也不是账号选择参数。

在 `launcher/` 下执行：

```sh
target/release/cols-launcher plan --config profiles/dedicated-a.local.json
target/release/cols-launcher launch --config profiles/dedicated-a.local.json
```

这是通用启动命令 `launch`，与 `native-launch` 不同。`plan` 只检查文件和启动条件，不验证服务器连接、证书或账号资料。通用模式跟踪直接启动的进程；若使用的 Wine 启动包装器会提前退出并分离游戏进程，应采用已有专用联调启动方式，不能套用本机原生模式的进程恢复保证。

第二个客户端使用另一份配置、根目录、prefix 和状态目录，并选择服务器已有的另一个登录身份。独立玩家创建默认输出独立服务资料根目录；不会自动把新玩家合并到这台服务器的 `--service-data-dir`。当前没有通用账号导入命令，不要把本机玩家列表当作服务器账号列表。

进入同一服务后，A 创建已准备的战术房间，B 在原生房间列表查找并加入。真人准备完成后共同开局；目标人数为 4 时，A 单人是 1 真人＋3 Bot，B 加入后是 2 真人＋2 Bot。结算后可在同一服务进程中再次准备开局。

## 部署到 Linux

所有构建在开发机完成，再上传 Linux 成品；云主机只部署运行。已验证的目标为 Linux x86_64、glibc 2.35、OpenSSL 3，开发机使用 Rust 1.96.0、Zig 和 cargo-zigbuild 交叉编译，并为该目标准备匹配的 OpenSSL 头文件和库。构建环境与边界见[远端验收](REMOTE_LINUX_ACCEPTANCE.md)。

部署目录可以整理为：

```text
/srv/codol/
  bin/codol-server
  tls/server.pem
  tls/key.pem
  service-data/
  maps/default.json
  maps/catalog.json
  maps/...
  captures/
```

上传前后核对可执行文件 SHA-256，在 Linux 检查目标架构和动态库：

```sh
file bin/codol-server
ldd bin/codol-server
bin/codol-server serve-auth --help
```

从部署目录执行上面的 `serve-auth` 命令，将程序、证书、服务资料和地图参数分别改为部署路径即可。Linux 服务端不需要 Wine、显示器或游戏窗口，但仍需要导出的地图数据。确保运行账号能读取证书／地图、写入服务资料和捕获目录。

### 远端客户端链路

当前代码只监听回环地址，远端接入需要转发 TCP 和 UDP。AUTH／大厅可以通过 SSH TCP 转发，例如：

```sh
ssh -N -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:39001:127.0.0.1:39001 \
  -L 127.0.0.1:39002:127.0.0.1:39002 \
  YOUR_SSH_HOST
```

`YOUR_SSH_HOST` 使用保存在本机私有 SSH 配置中的目标别名；客户端仍指向本地回环端口，服务端也继续监听远端回环端口。

这条命令只接通两条 TCP 通道。STUN 3074／3078 和游戏 UDP 39004 还需要独立的 UDP 转发，必须保留客户端映射与回复路径；SSH `-L` 和启动器 TCP 转发器都不承担该功能。双端远端验收使用专用 SSH 数据帧转发工具，该工具位于私有联调环境，没有作为通用部署工具随仓库提供。因此当前公开工具尚不能一条命令完成远端双端接入，不能仅开放防火墙或填写服务器 IP 就宣称可公网游玩。

## 停服、持久化与排查

前台按 Ctrl-C 停止服务。使用进程管理器时，将停止信号设置为 SIGINT，并等待进程完成清理；当前 CLI 的协作关停入口监听 Ctrl-C。停服会断开所有玩家，重新启动不会恢复尚未完成的对局。

账号、公私档案、库存、钱包和经验回执位于 `--service-data-dir`，与捕获目录分开。备份或迁移账号时先停服，保留完整服务资料；不要对同一根目录并行运行两个服务进程，账号写锁只协调同一实例内的业务。

| 现象 | 检查方向 |
| --- | --- |
| `must bind loopback` | 当前不支持公网／局域网地址直接绑定；保持回环并使用已验证的转发链路 |
| 地址已占用 | 检查本机原生启动器、`play.py` 或默认观察器是否也在管理这些端口 |
| AUTH 可连接但无法进图 | 等待游戏服务就绪，检查地图引用、实际分配端口与 UDP 链路 |
| 远端大厅正常，游戏连接超时 | 两条 TCP 转发不足以完成 STUN／游戏 UDP，检查独立 UDP 转发 |
| 两端无法同时在线或串用存档 | 检查 `dw_username` 是否不同，以及对应账号是否在同一服务器资料中 |
| 列表里看不到房间 | 确认两端连接同一服务、客户端版本及房间条件一致，房主仍在线 |
| 没有 Bot | 检查 `--room-bot-fill`、战术模式、地图准备及房主接纳名单 |
| CLI 已运行但没有游戏窗口 | 独立服务器本来不启动客户端，需在客户端机器另行启动游戏 |

共享多人中的中途配装、个人回放、经验和完整装备仍有独立验收边界，见[多人验收](MULTICLIENT_ACCEPTANCE.md)。本地一体启动则按[使用教程](USER_GUIDE.md)操作。
