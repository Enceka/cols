# 地图快速验收

已准备地图采用后台批量模拟和原生短局两类检查。后台检查每次改动都可运行；新地图第一次接入必须补做原生加载、移动与结算实采。共享回放、名片、技能HUD和动作的完整体验按专项复核，不要求每张图都重新冷启动并跑十分钟长局。小时级稳定性和完整视觉／听感仍独立验收。

## 一次配置与自动执行

```sh
python3 tools/prepare_map_batch.py prepare \
  --catalog research-output/native-tdm-map-catalog.json \
  --inventory research-output/full-map-coverage-audit.json \
  --output research-output/map-batch-config-new

python3 tools/materialize_map_batch.py \
  --queue research-output/map-batch-config-new/queue.json \
  --template research-output/tdm-crossfire-state.json \
  --inventory research-output/full-map-coverage-audit.json \
  --normal-experiment research-output/gamehub/client-EXPERIMENT/experiment.json \
  --output research-output/map-batch-run-new
```

所有输出目录必须不存在；`--normal-experiment`必须是正在运行的正常游玩实验manifest，不能传`current-experiment.json`包装文件。脚本核对本版SHA、客户端PID及其父harness的真实命令后停止正常会话，等待证书清理；自动在`client-work/map-batch-<输出目录名>`建立独立服务资料副本。也可指定已有独立`--service-data-dir`。诊断目录不能指向正常资料或其子目录。

26条地图身份一次生成共享7500分／600000ms配置和逐图任务；只有本图真实准备进入ready catalog。观察路径但尚无准备标为`needs_resources`，没有观察到有效路径标为`needs_resource_audit`。队列不会给缺资源图填写其它图的CRC或碰撞。

执行顺序是：后台预检并隔离坏候选 → 自动完成资源准备尝试 → 后台批量检查 → 通过项的原生短局 → 正常会话恢复。资源阶段使用不变的、后台通过的采集目录，新候选不会阻断后续采集。完整资源捕获优先复用；旧碰撞缺少BSP世界根证明时只补采碰撞，保留其它输入，详见`WORLD_COLLISION_SCOPE.md`。只保留原始捕获时复用原始阶段，再初始化补采。`--cache`指定已有采集目录，`--maps`限定定向准备，`--native-maps`限定原生复核；`--native-new-only`只对本批新准备图跑原生短局，避免重复已有覆盖，不能与`--native-maps`同时指定。`--skip-collection`只验已有准备，`--skip-native`明确只做资源／后台检查。

后台目录加载失败时自动拆成单图，继续其余图。原生失败保存`*-failure-state.json`、可用的map-check／replay诊断及服务日志；自动停止该诊断会话。尚未进入本图、仍处于前端的失败允许一次独立会话重试，保留首次证据；其它失败继续下一图，正常通过项复用当前客户端。结算返回必须确认真实12人房间，按钮过早未生效时最多再试两次。资源阶段失败保存该阶段状态和日志。没有盲点弹窗恢复、不自动截图、不写客户端内存。最后核对正常资料／准备文件哈希，恢复原catalog、时限、账号和背包，写`normal-restoration.json`；停止的诊断实验及证书恢复写`diagnostic-cleanup.json`。失败使执行返回非零；尚未准备项仍保留在`pending_resources`，不能把任务已配置或后台通过称为全地图原生通过。

`acceptance/result.json`分别记录`ready_server_passed`、每图`native_passed`、定向失败清单和未准备项。只有26张地图均有真实准备并通过原生检查时，`all_maps_passed`才为true。证据文件保留原始来源，不把准备目录当可迁移资源包。

## 后台批量检查

```sh
target/release/codol-server verify-maps \
  --catalog research-output/native-tdm-map-catalog.json \
  --output research-output/map-verification-new.json \
  --seconds 60
```

输出路径必须不存在。命令读取真实准备文件、碰撞、导航、模型和武器，以固定种子按50ms步长模拟每图60秒；支持15～120秒。每图检查11个Bot的身份、坐标、模型与枪械，至少8个Bot在存活期间累计移动128单位，以及交战击杀、死亡／重生周期和三个真实参与者的地面排名场景。死亡或重生传送不计为行走。单图模拟失败写入报告，继续其它图；报告失败时命令返回非零。准备文件加载失败直接报错。

不读取账号、库存或钱包，不启动客户端，不修改正常7500分／600000ms配置。报告明确标记`native_client_used=false`，记录每图CRC、随机种子、击杀／重生和耗时；资源加载与模拟耗时分别列出。

`research-output/fast-four-map-podium-proof.json`中的四图全部通过：11个Bot均达到移动门槛，Afghan／Bog／坠机／安全屋分别有13／29／22／39次交战击杀和12／28／22／36次重生，四图排名场景均生成成功。资源加载1000ms、模拟956ms、合计1957ms；另一次实测含资源加载5361ms，见`fast-four-map-acceptance-proof.json`。这是后台功能检查，不是原生视觉验收。

## 原生短局批量检查

先生成独立诊断目录，保留正常文件和资源引用：

```sh
python3 tools/verify_native_maps.py \
  --catalog research-output/native-tdm-map-catalog.json \
  --prepare research-output/map-smoke-new
```

诊断文件为1000分／60000ms。使用独立服务资料副本启动一次研究客户端，harness必须显式指定诊断catalog及`--service-data-dir`，不能使用正常`client-work/server-data`。单独运行该原生工具不负责停止或重启客户端；完整批量入口负责诊断会话及正常恢复。

已有合适的运行实验时：

```sh
python3 tools/verify_native_maps.py \
  --catalog research-output/map-smoke-new/catalog.json \
  --experiment research-output/gamehub/client-EXPERIMENT/experiment.json \
  --maps mp_bog_sh mp_crash_tropical mp_killhouse_sh \
  --output research-output/native-map-smoke-new.json
```

`--maps`按给定顺序运行；未列地图保持未检查。原生创建房间、箭头选图、准备、有限移动和结算返回只使用正常系统输入。复用同一客户端、同一房间，不自动截图。结算按钮在state0后稍晚才接受点击；保留的12行统计不是当前页面标记，工具等待切换后返回，再核对箭头变化。

箭头点击使用控件中心；自动选图保持一个方向，遇到边界或完整循环后最多反向一次，并在每步核对真实地图和人数。客户端离开房间状态后立即停止房间输入，保存`map-selection-*.json`中的路线、窗口尺寸及实际点击坐标；这些记录无需截图。

只读`--map-check`探针限定读取本图身份、12名角色的现有组合模型／动画树及本图小地图贴图；不遍历全部资源、不调用客户端函数。坠机使用真实`compass_map_mp_crash_dlc`。批量工具检查本图CRC、同一条命的玩家移动、人物资源和非默认贴图，并连续采样原生游戏／回放状态。输入前重新取可玩状态，W／D／S每次最多0.7秒，至少一次同生命周期移动8单位；不能把重生传送当成移动。结算必须是本图ID、12名参与者、单个本地玩家及非零击杀。读数切换期间最多重试三次；仍失败则保存失败报告，不标为通过。

报告保存原始样本路径、普通游戏／回放／中场帧数和AAR。回放、前三名画面及声音的完整观感不是本工具的通过条件。单独原生工具遇到选择循环、初始化失败、缺少模型或默认小地图会保存失败并中止；完整批量入口隔离该会话并继续其余地图。

快速验收实验、正常账号恢复及剩余边界见`MAP_RESOURCE_PREPARATION.md`和`REVIEW_PENDING.md`。

`research-output/quick-native-podium-two-map-proof.json`记录`client-1791774867798`同一客户端／房间的Bog→坠机两局，合计271.91秒。Bog155.37秒含创建／选图，坠机116.54秒；两图均读到12名角色、非默认1024贴图、同生命周期移动112.62／133.37单位及12行结算。最终回放与PM_INTERMISSION6有原生采样，排名场景打开／ACK／关闭和结算有服务日志；两局AAR胜负／比分与服务端逐项一致。工具没有截图，不将这些状态样本当作完整演出验收。

完整检查`fast-map-podium-final-check.log`通过444项Rust、61项Python及fmt／Clippy；优化构建见`fast-map-podium-release.log`。隔离诊断结束时证书已恢复，正常账号文件逐项哈希不变，见`fast-map-account-restoration-proof.json`。Afghan和安全屋未包含在这次原生两图批次中；四图后台检查及此前各图实机证据分别保留。

批量配置证据为`map-batch-automation-queue/queue.json`，26项一次生成。`map-batch-automation-acceptance`复用杂草丛生完整采集，九图后台通过；贫民窟和杂草丛生原生通过，山景别墅首次因上一局前端返回而未进入地图。`map-batch-automation-native-v2`随后在同一客户端／房间完成山景别墅→贫民窟，合计362.65秒，两图12人角色／结算及同生命周期移动130.73／132.75单位；AAR与服务日志一致。首次失败保留，不能据其认定地图资源坏损。批量工具无截图；返回转换另有一次人工定向截图，捕获到过渡黑帧，不作为完整视觉验收。

两批结束均自动恢复正常客户端，原账号／准备哈希一致，库存6772；正常重登仅允许用户设置文件自然更新。证据为各批的`normal-restoration.json`和`map-batch-nine-map-admission.json`。这些证据中的九图覆盖已扩展至下述13图；全地图尚未通过。

`map-expansion-next-batch`新增锈迹之地原生短局158.98秒。其余九张选图失败的状态／日志独立保留；航站楼原准备的后台错误由世界碰撞范围修正恢复，见`WORLD_COLLISION_SCOPE.md`。`world-collision-native-acceptance`同一客户端／房间完成核弹镇→货轮，151.74／143.03秒；`world-collision-terminal-native`完成航站楼161.70秒。三图均确认12人模型／结算、同生命周期移动、非默认小地图，AAR与服务日志一致。正常配置及账号恢复、四图准入证据见`world-collision-thirteen-map-admission.json`。完整代码检查`world-collision-final-check.log`通过444项Rust、80项Python及fmt／Clippy；只读探针构建见`world-collision-probe-build.log`。
