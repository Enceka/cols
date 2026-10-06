# GameHub 动态调试与首次 AUTH 请求

日期：2026-10-06。用户指定优先使用 GameHub 的兼容层；Parallels 作为对照环境。

本轮已让原始样本的研究副本在 Mac 上执行，并依次取得配置读取、真实 TLS ClientHello、完成 TLS 协商、发送 HTTPS AUTH 请求的证据。尚未返回有效认证票据，也没有进入大厅或验证背包。

以上为早期实验记录。后续已解决相对 CA 路径问题，并用 Rust 完成票据与协议 210 握手；当前状态以 [AUTH_LOBBY_PROGRESS.md](AUTH_LOBBY_PROGRESS.md) 为准。

**实际环境**

GameHub 在本机已有两个安装完成的引擎，元数据位于 `~/Library/Application Support/com.gamemac.www/wine-engine/container/wine_installations.json`：

| 引擎 ID | 本地名称 | 实际 --version | 本轮使用 |
| --- | --- | --- | --- |
| 10000033 | wine-proton_10.0 | wine-10.0-ge3bfd9eaa79 | 完成初始化、客户端运行与 TLS/AUTH 捕获 |
| 10000073 | wine-proton_11.0 | wine-11.0-g72ab2268eb3 | 只核对版本及程序类型，尚未启动客户端 |

两者的 wine 都是 macOS Mach-O x86_64 程序，安装中有 i386-windows 与 x86_64-windows 模块。这里使用的是 GameHub 的 macOS 定制引擎；上游 Valve Proton 的主要运行目标仍是 Linux。[Valve Proton 说明](https://github.com/ValveSoftware/Proton)

实际工作容器为 `client-work/wine10-prefix`，客户端为 `client-work/CODOL`。客户端通过同卷 `cp -cR` 创建可写克隆，主程序 SHA-256 与原始样本一致。GameHub 原有游戏容器及其安装元数据未修改。本轮没有经过 GameHub 图形界面配置图形组件，也没有把引擎默认组件之外的优化设置假定为已生效。

Parallels 的 Windows 目前为 running、Parallels Tools 已安装，系统为 Windows 11 Arm（10.0.26200.8037）。`prlctl exec` 默认进入 SYSTEM/session 0；`--current-user` 能进入当前用户/session 1。该虚拟机尚未运行 CODOL，本轮没有向其复制客户端或更改共享目录。

**实验结果**

| 实验目录（research-output/gamehub/ 下） | 设置 | 结果 |
| --- | --- | --- |
| attempt-001 | 引擎 10、50 秒、文件调用日志、title_id=0 | 主程序/原生 DLL 加载，读取 dwauth.ini、iips.ini，启动 IIPSHostApp 并打开 .ifs；时间盒结束，未获 AUTH 数据 |
| client-1791258054604 | 引擎 10、120 秒、TCP 观测器 | endpoint_b 捕获真实 70 字节 TLS ClientHello，10 秒无响应后由观测器关闭 |
| client-1791258301729 | 本地 TLS 1.0、自签名实验证书、未修改客户端 CA | 连接在 ClientHello 前结束，服务记录 unexpected EOF；不足以认定它是证书拒绝 |
| client-1791258438998 | 同上，临时把实验 CA 加到研究副本 cacert.pem | 完整 TLS 握手，捕获 336 字节 HTTP 请求，JSON body 162 字节、title_id="0" |
| client-1791258679573 | 仅把配置末项改为 0x000003EA | 再次完整握手，捕获 338 字节 HTTP 请求，JSON body 165 字节、title_id="1002" |

所有限时客户端实验结束时都由工具关闭自己容器内的 Wine 进程和本地监听器。`stopped_by_harness=true` 是人为结束实验，不表示客户端崩溃。原始记录包含时间、启动参数、引擎、配置以及退出方式。

两次使用实验 CA 的执行结束后都恢复了研究副本的 cacert.pem；已与原件逐字节比较，相同 SHA-256 为 `b9262687fdcf2cce776d54ffab891f791b0d9e9f3e155a03d4497020d0c1eab9`。实验 CA 没有导入 macOS、Windows 或其他 GameHub 容器的信任库。

日志还显示 TerSafe.dll 实际被加载、`dwUin=0` 被 SDK 记录为无效参数，以及 IIPS/组件产生了其他网络流量。这些都不是本地 AUTH 成功的证据。图形层出现过 OpenGL framebuffer 错误，声音组件也有 COM 错误；本轮证明了可运行到认证请求，尚未证明完整渲染、音频和大厅界面正常。

**已经确认的协议事实**

1. dwauth.ini 第三、第四字段（endpoint_b.host/port）指向本地 HTTPS `/auth/` 服务。第一、第二字段的业务角色仍待后续验证。
2. 配置第五字段是 title_id。0 与 0x3EA 两次实验分别得到 JSON 字符串 "0" 与 "1002"；字段映射已经确认，1002 是否为本构建应采用的部署值还未验证。
3. 首个 ClientHello 的 record_version 与 client_version 均为 0x0301，密码套件有 11 项，session_id 为空，compression 仅 null，没有扩展块。仅凭 record_version 本身不足以判断协议版本；这里也检查了 ClientHello 内部字段。[TLS 1.0 结构定义](https://www.rfc-editor.org/rfc/rfc2246)
4. 实验服务以 TLS 1.0、AES128-SHA 完成协商后，客户端发送 HTTP/1.1 `POST /auth/`，Content-Type 为 application/json，并带 X-TransactionID、Content-Length 和 `Expect: 100-continue`。实验服务未发送 100 Continue，客户端等待后仍发送 body；后续 HTTP 实现应正确处理该头。
5. 首个请求 auth_task 是字符串 "10"。iv_seed、title_id、identity、extra_data 也都是 JSON 字符串。extra_data 的内容又是一层 JSON，其 version 和 user_id 仍是字符串。
6. identity 的 "356c4bc3" 同时出现在主程序 VA 0x00c07e5c，并由 VA 0x00a59f15 的指令引用，是当前样本的固定字符串。它的完整业务含义尚未确认。

原始 POST 中的 user_id 超过有符号 64 位整数范围。Rust 已使用 u64 解析；后续 SQLite 持久化必须选择能完整保存无符号 ID 的表示，例如十进制文本或固定宽度字节，不直接截断为 i64。

脱敏结构示例：

```json
{
  "auth_task": "10",
  "iv_seed": "1",
  "title_id": "1002",
  "identity": "356c4bc3",
  "extra_data": "{\"version\": \"0\", \"user_id\": \"2\"}"
}
```

示例中的 iv_seed/user_id 为合成值。原始认证数据留在忽略目录；可提交的 fixtures 仅含公开 ClientHello 和脱敏 AUTH 结构，来源标注在 fixtures/protocol/README.md。

**现有工具与重现步骤**

以下命令在 server/ 下执行。已有研究副本时跳过 prepare；该动作拒绝覆盖已有目录。

```sh
python3 tools/gamehub_run.py list
python3 tools/gamehub_run.py prepare
python3 tools/gamehub_run.py init --seconds 180
python3 tools/gamehub_run.py probe --seconds 30
python3 tools/gamehub_run.py client --seconds 120 --title-id-hex 0
```

init 的首次 DLL 注册可能较慢，本轮第一次 45 秒时间盒不足，第二次延长后退出码为 0。probe 是独立的 32 位 cmd 检查入口，尚未作为本轮独立实验执行；真实 PE32 客户端已执行到 TLS 请求。

--file-trace 增加文件调用记录；--crossproxy 可复现启动器随后启动 CrossProxy 的动作，本轮成功捕获 AUTH 的实验均没有启用它。该事实只说明捕获首个请求不需要这个步骤，不代表完整大厅不依赖 CrossProxy。

TLS 探针使用独立的 OpenSSL 进程，属于研究工具，**不是已经完成的 Rust TLS 服务端**。安装位置由本机已有 OpenSSL 决定，本轮为 Homebrew OpenSSL 3.6.4。其 TLS 1.0 参数只作用于绑定 127.0.0.1:39002 的本地进程，没有改全局 TLS 策略。[OpenSSL s_server 文档](https://docs.openssl.org/3.6/man1/openssl-s_server/)

创建短期实验证书（私钥与证书放在被忽略的 .tools，已有文件时先选择新的目录或确认实验用途）：

```sh
mkdir -p .tools/tls-lab
/opt/homebrew/bin/openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 2 \
  -subj '/CN=127.0.0.1/O=CODOL Local Research' \
  -addext 'subjectAltName=IP:127.0.0.1' \
  -addext 'basicConstraints=critical,CA:TRUE' \
  -keyout .tools/tls-lab/key.pem -out .tools/tls-lab/server.pem
python3 tools/gamehub_run.py client --seconds 75 --title-id-hex 3ea --tls-probe --trust-lab-cert
```

工具只允许带所有权标记的项目容器/副本，保存实际参数并限制运行时长和日志总量，最后定向停止当前 WINEPREFIX。--trust-lab-cert 临时修改的仅是该研究副本的 CA 文件，并在 finally 中恢复；工具也保存 cacert.before.pem，供进程被强制终止后的恢复使用。

Rust 目前增加的是离线解析器：

```sh
sh tools/cargo.sh run --locked -p codol-server -- inspect-tls --input fixtures/protocol/clienthello-3.3.3.3.bin
sh tools/cargo.sh run --locked -p codol-server -- inspect-auth --input fixtures/protocol/auth-task10-sanitized.json
```

inspect-tls 只接受一个完整、已重组的 ClientHello record，不能把 TCP 的一次 read 当成一个 record。inspect-auth 接收 JSON body 而非整段 HTTP；保留双层字符串表达，输出摘要不包含用户 ID 和 iv_seed。两个真实实验 body 均已由 Rust 命令解析通过。

**响应逆向的下一入口**

已沿新发现的字符串检查本样本中的代码，以下是静态证据，尚未用响应注入验证：

| 首选 VA / RVA | 观察 | 后续任务 |
| --- | --- | --- |
| 0x00a59df0 / 0x659df0 | 构造 auth_task、iv_seed、title_id、identity、extra_data，拼接 `https://%s:%d/auth/` | 明确任务 10 的完整状态和 extra_data 来源 |
| 0x00a598b0 / 0x6598b0 | 检查 HTTP status 200，然后读取 auth_task、code、iv_seed、client_ticket、server_ticket 等字段 | 恢复返回任务编号、错误与成功分支及校验规则 |
| 0x00a59a24 / 0x659a24 | 将 code 与 0x2BC（700）比较 | 不能直接把 700 当成已验证可用的成功响应 |
| 0x00a59aec / 0x659aec | client_ticket 字符串送入一个解码调用，输出长度与函数参数比较 | 确认解码算法、票据大小和后续校验 |
| 0x00a59b6a / 0x659b6a | server_ticket 送入同一调用，返回长度需为 0x80 | 128 字节是当前静态检查线索；结构及密码算法未恢复 |

重现代码：

```sh
.venv/bin/python tools/analyze.py native --rva 0x6598b0 --size 0x540
.venv/bin/python tools/analyze.py native --rva 0x659df0 --size 0x410
```

下一步先把 AUTH 传输、HTTP 读取和真实请求分发接入 Rust，再恢复票据及客户端校验。当前 M0 的“可重复启动到联网”已有证据；M1 的“客户端接受认证响应并进入档案/大厅初始化”仍未通过。
