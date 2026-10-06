# 启动链与 dwauth.ini：静态发现

日期：2026-10-06。本文件记录固定样本的静态分析；同日后续动态实验已确认 endpoint_b=HTTPS AUTH、末字段=title_id，见 [GAMEHUB_DEBUG.md](GAMEHUB_DEBUG.md)。样本哈希见 [CLIENT_BASELINE.md](CLIENT_BASELINE.md) 和 fixtures/client-3.3.3.3.json。

**离线启动器**

使用 dnfile 解析 .NET 元数据、dncil 解码 12 个方法，得到两条关键方法：`Form1::.ctor`（MethodDef 0x06000001，方法体 RVA 0x2050）与 `Form1::Form1_Load`（MethodDef 0x06000002，方法体 RVA 0x20fc）。以下 IL 偏移从方法代码首条指令计数，不包含 12 字节方法头。

| 位置 | 观察 |
| --- | --- |
| .ctor IL_0043 | 默认游戏参数为 `-q 0 -src=tgp -game_id 29 -zone_id 83886082` |
| .ctor IL_004e / IL_0059 | 主程序和 Base.dll 文件名分别赋给字段 |
| Form1_Load IL_0035 | 检查 Base.dll 是否存在，设置 FLAG_TVM |
| Form1_Load IL_004c–IL_0051 | 检查 `IIPS\IIPSHostApp.exe` 是否存在，设置 FLAG_IIPS |
| Form1_Load IL_006e | 检查游戏主程序是否存在，缺少时提示并退出 |
| Form1_Load IL_01bc / IL_01ce | 设置 ProcessStartInfo.FileName 与 Arguments |
| Form1_Load IL_01e4 / IL_01f1 / IL_0202 | 工作目录为启动器 BaseDirectory，UseShellExecute=true，Verb=runas |
| Form1_Load IL_020a | 调用游戏进程的 Process.Start |
| Form1_Load IL_0232–IL_0237 | 游戏启动尝试后检查 `Cross\CrossProxy.exe` |
| Form1_Load IL_0258 / IL_0269 / IL_0289 | CrossProxy 文件名、参数 `41c221af 0 0 0 5000002`、工作目录 BaseDirectory+Cross |
| Form1_Load IL_0290 | 调用 CrossProxy 的 Process.Start |

在已分析的方法里，Base.dll 和 IIPS 文件的存在影响标志及通知文本；没有发现由启动器直接加载 Base.dll 或启动 IIPSHostApp 的调用。因此“缺少 Base.dll”不构成本启动器阻止游戏 EXE 启动的充分条件。游戏及其子组件自身是否要求这些文件仍待运行验证。

启动器还让嵌入式 WebBrowser 导航到一个外部页面；该地址本轮只从样本读取，没有访问。没有在这些方法中发现修改游戏文件、注入模块或实现服务端认证的行为。不能把这个启动器称为已恢复的离线登录服务。

路径细节：File.Exists 检查使用相对路径，而启动进程的 WorkingDirectory 明确设置为启动器目录。原启动器当前位于 CODOL 目录之外；若以后测试它，需要同时明确工作目录和放置位置。独立研究启动方式应根据以上实际动作构建，并用动态日志确认 CrossProxy/IIPS 的真实依赖。

重现：`.venv/bin/python tools/analyze.py launcher`。输出包含样本哈希、每条指令 RVA、IL 偏移及解析后的 .NET 成员引用，不运行程序集。

**原生配置读取**

主程序固定 ImageBase 为 `0x00400000`。本文件统一写明地址类型；即便部分段的文件偏移恰好等于 RVA，也不可将二者混用。

`dwauth.ini` 字符串的文件偏移/RVA 为 `0x877864`，首选 VA 为 `0x00c77864`。在可执行节发现该 VA 的绝对地址引用，进一步沿指令读取确认 `push 0x00c77864` 位于 VA `0x00641f01`，其所在启动函数存在标准函数序言 VA `0x00641e10`。

在 RVA `0x241efc..0x241f85` 的连续指令中，先传入 `r` 与文件名，再检查返回的句柄是否为零；成功分支顺序传入以下格式和目标地址，调用同一个可变参数读取例程，最后关闭句柄。该调用形态及格式字符串符合 fopen/fscanf/fclose；CRT 函数名称仍为静态推定，字段顺序和操作数则直接可见。

| 顺序 | 格式字符串 VA / 原始内容 | 目标全局 VA | 调用点 VA | 当前命名 |
| --- | --- | --- | --- | --- |
| 1 | 0x00c77870 / `%59s\n` | 0x03747198 | 0x00641f2b | endpoint_a.host |
| 2 | 0x00c77878 / `%hu\n` | 0x037489e4 | 0x00641f3e | endpoint_a.port |
| 3 | 0x00c77870 / `%59s\n` | 0x03746910 | 0x00641f51 | endpoint_b.host |
| 4 | 0x00c77878 / `%hu\n` | 0x0374911c | 0x00641f64 | endpoint_b.port |
| 5 | 0x00c77880 / `%x\n` | 0x03749110 | 0x00641f77 | title_id（后续动态差异实验确认） |

因此这是五个由空白分隔的字段，而不是带 `[section]` 和 `key=value` 的普通 INI。两组地址各占一行、最后一个十六进制值占一行，符合上述扫描格式；后续真实客户端已读取该配置并连接指定端点。

`crates/protocol/src/bootstrap.rs` 按这个证据实现研究配置的解析/生成，并主动拒绝过长主机名、非法编码、零端口、溢出整数和额外字段。它比 C scanf 对错误输入的行为更严格，目的在于生成确定格式，未声称模拟所有原始错误行为。

重现配置读取指令：

```sh
.venv/bin/python tools/analyze.py native --rva 0x241efc --size 0x8c
```

**后续交叉引用线索**

下面的地址已经查看过部分指令，但没有完成全函数控制流和动态分析，属于下一轮入口：

- VA `0x006d2f70`（RVA `0x2d2f70`）：读取 endpoint_b.port 和 context_value，将 endpoint_b.host 传入对象构造路径，后续出现 `cacert.pem` 和相关设置调用。两个候选构造目标为 VA `0x00a47b10`、`0x00a58620`。这提示 endpoint_b 可能参与认证/TLS 路径，但尚不作为正式协议命名。
- VA `0x006d38d0`（RVA `0x2d38d0`）：分别把 endpoint_b.host 与 endpoint_a.host 送入两个对象的方法；有地址解析特征，具体对象类型待确认。
- VA `0x006d3c90`（RVA `0x2d3c90`）：读取 endpoint_a.port，组装地址及其他状态后调用 VA `0x00a443d0`。需要继续核实与大厅会话建立的关系。
- VA `0x004b05c0..0x004b0766`：存在另一组对相同全局变量的写入，并涉及五、七、九字段分支。需追踪其输入来源与调用时机，确认是否覆盖 dwauth.ini。

后续动态证据已确认 B 是 HTTPS AUTH，末项映射为 title_id；A 的职责以及有效部署 title_id 仍需验证。其余任务编号、票据格式和密码常量继续按样本证据恢复。

**仍未完成**

后续已完成 Mac GameHub 启动到 AUTH、配置读取、TLS 1.0 和首个 task-10 JSON 的动态观察。会话密钥、有效认证票据、档案/背包/大厅任务仍未恢复，尚未通过客户端登录验收。
