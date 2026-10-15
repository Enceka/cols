# 本地新玩家

启动器提供昵称、初始等级 1～70 和全物品解锁选项。服务端 `backend/players` 使用外部初始模板创建独立资料，不复制已有账号的私有战绩、钱包或结算历史。昵称允许重名，目录使用随机 128 位玩家 ID。创建设置保存在 `profiles/<title>/<native_uid>/player.json`；日常启动不重新写入初始等级和库存。

默认角色、配装与物品表来自用户本机的 3.3.3.3 资料。开发者准备安装环境时执行：

```sh
python3 tools/prepare_player_template.py \
  --assets /path/to/verified/client-assets.json \
  --public-profile /path/to/initial/public.bin \
  --user-id NATIVE_UID \
  --output client-work/server-data/new-player-template.json
```

输入公共档案必须是已校验、未混淆的初始配装，不能使用进阶账号快照作默认模板。准备器核对客户端摘要、public v233 定义、CRC、125 项投影和物品表，从当前角色与第一套核心装备提取初始所有权，并设置已核对的本地大厅引导完成位。模板不含私有档案、钱包或结算回执；原始资源、真实 UID 和模板均留在忽略目录。启动器运行时只调用 Rust 服务端，无 Python 依赖。

```sh
target/release/codol-server create-local-player \
  --source-data-dir client-work/server-data \
  --template client-work/server-data/new-player-template.json \
  --output-parent /path/to/launcher-state/players \
  --title-id 1002 --nickname "玩家一" --level 20 --unlock-all
```

命令在随机命名的暂存目录中生成完整资料，成功后 rename 为最终目录，返回 JSON 中的 `player_directory`。共享 publisher、商城及已有 progression 规则复制到新根目录；私有档案从零数据生成，rankVersion=4、级内经验=0，校验普通等级的解锁下限。库存按选项使用初始装备或完整物品表，CP／GP 为零。新玩家没有 `progression.json`；后续结算仍由经验服务维护。失败清理本次暂存目录，原服务资料保持原状。

全物品解锁仅授予物品所有权，不批量篡改未知 unlockFlags，不把武器经验设为满级，也不代替剧情或其它 UI 的实际门槛。人物等级与物品授予相互独立。认证票据的名称使用本版 SDK 的 Base64 编码，游戏连接完成身份校验后使用保存的昵称。

每个新玩家生成独立的 ASCII 登录标识 `c` 加玩家目录 ID 的前 14 个十六进制字符。启动器把该标识传入原生 `dw_username`；显示昵称仍可重名。服务端按 `Tiger192(lowercase(Base64(login_name)))` 的前八字节小端值创建原生 UID，资料按该 UID 保存。模板 UID 只用于验证来源，不作为新玩家身份复用。旧配置缺少 `login_name` 时保留原生默认登录行为。便携存档握手尚未实现。自动验证覆盖等级边界、同名数据隔离、完整／初始库存、失败清理、原资料保留、认证昵称及对局身份校验。真实 release 启动器／服务端 CLI 已验证同名账号分别生成 1 级＋8 件装备和 70 级＋6772 件物品，配置读回成功，原账号四个核心文件哈希保持一致；使用合成客户端做只读前置检查，没有启动游戏。尚未进行新玩家的原生登录验收。
