# 剧情测试解锁与货轮资源采集

本版可见的10个剧情关卡已在独立测试账号中解锁。解锁工具将本版`mp/unlockFlags.csv`中的13个`cf_ch_coop_*`标记对应的`unlockFlagsNew`数值提升至至少1，保留更高值。它只修改测试副本的私有档案，重新计算CRC/投影并保存完整账号备份；普通账号、背包、钱包和奖励记录不作为解锁写入目标。关闭版本列的地图仍隐藏。测试解锁属于模拟完成标记，不能用作实际通关或解锁持久化验收。

客户端实际读取`mp/playersecuredata.def v19`的`unlockFlagsNew` Int16数组，通过`0x4a4ca0`比较最低值。旧`unlockFlags`布尔数组不能单独恢复菜单准入。`tools/prepare_story.py`现在同时记录当前数值字段、最低值1及旧布尔字段，避免混淆。

## 首关原生证据

`coop_training_mp_cargoship / coop`已通过真实首关入口完成：无`21/1`的随机security ID/key分配、`mmbook`、原生`mmstart`、随机P-224握手、认证DATA、challenge/connect、stats上传及gamestate资源初始化。初始化采集状态为9，没有发布出生或玩法快照。

原生开局使用单人容量、难度0，最后的`2654`参数为空。`story_capture.rs`只为这个已捕获的空参数形式补校验，其它缺值、重复设置和不支持的命令继续拒绝。首关加载的底图资源名是`maps/mp/mp_cargoship.d3dbsp`；剧情标识与共享BSP标识必须分开保存，不能按TDM地图名过滤剧情实体。

原生实测与独立复算一致：NCS CRC **2662333828**，公共dvar CRC **3451066449**，59个有效常量资产、3515条常量。63条捕获记录包含默认/不参与校验条目，不能把63当作CRC资产数。

| 采集项 | 本地结果 |
| --- | --- |
| 地图实体 | 470302字节，3526个实体块；按首关`script_specialopsname`精确筛选后3440个，含共用实体 |
| 世界碰撞 | 24431个刷子，61506个三角形，按本版BSP世界叶引用筛选 |
| 武器 | 194个注册武器记录及本版装配/属性资料 |
| 角色与模型 | 272个模型记录、角色表、动画与装配资料 |
| 原始客户端脚本/规则 | 4个FX GSC、玩家动画规则、首关animlist及客户端常量基线文件，共8个相关rawfile |

FX脚本包括共享货轮和首关的效果入口/createfx内容。**完整任务GSC、敌人/NPC行为、任务触发、检查点、胜败与通关提交仍未验证**。`animlist`与实体数据可以提供依赖线索，不能等同任务执行逻辑。客户端常量基线文件也不代表服务器已实现角色行为。

完整捕获位于忽略目录`research-output/story-resources-20261014/cargo-initialized/`，`capture-summary.json`保存最小汇总，`verified-preparation/`保存复算后的准备文件。各捕获文件有内容哈希；原始脚本、资源、账号和握手材料不加入Git或发布。初始化前的等待记录保留在`result.json`；最终六项资源采集没有资源读取错误。此前仅传输采集的未初始化结果单独保留，不替代初始化证据。

## 工具与入口

- `tools/unlock_story.py`：要求`.codol-story-capture.json`标记的独立服务目录，核对固定客户端schema及账号归属；默认输出计划，`--apply`离线备份后只写指定剧情标记。
- `tools/ClientStory.cs`和`client_state.py --story-assets / --story-bundle`：仅使用VM_READ读取加载的资源，不调用客户端函数、不注入或写内存。bundle在已初始化状态9～10一次读取实体、常量、脚本、碰撞、武器和角色，减少重复启动探针错过加载窗口。
- `tools/capture_story.py`：验证实验PID/研究副本和固定探针哈希，等待加载、保存本地捕获及逐文件哈希。它不启动游戏或操作前台。
- `--story-resource-state`：必须与显式首关采集结合。只读取限定元数据并发送gamestate供本图初始化；专用加载器拒绝出生、碰撞文件、武器和其它世界字段，不进入默认TDM加载器。连接持续保活，没有玩法快照。

准备初始化元数据时，将`prepare_story.py`的`--constants`与`--map-entities`配对使用。输出`cargo-metadata.json`只含固定版本/首关身份、来源哈希、CRC与数量；它不导入账号、脚本或世界。

```sh
python3 tools/unlock_story.py --assets <只读schema捕获> \
  --service-data-dir <独立测试服务目录> --user-id <测试UID> --apply

python3 tools/client_state.py --build --prefix-name wine10-probe-roster-20261010
python3 tools/capture_story.py --experiment <活动experiment.json> \
  --probe .tools/ReadClientState.exe --output <新采集目录>
```

初始化采集的真实实验是`client-1791962675004`，会话已正常清理，连接配置和两处临时CA已恢复。只读源字段/脚本存在性、CRC复算、采集哈希和测试解锁的字节保留均已核对；独立工作树fmt/Clippy及完整Rust/Python检查通过。原生菜单、握手和资源初始化是本次实机范围，未执行完整剧情玩法。

本机启动还遇到公共资源`ff_init_common_gfx_V43.ifs`文件对象的系统`open()`阻塞。已保留此前目标，采用用户本机已有缓存的完整哈希一致内容创建新文件对象恢复启动，记录于`story-integration-preparation/resource-recovery.json`。没有修改EXE/DLL或资源字节，也没有由此推定文件系统根因。
