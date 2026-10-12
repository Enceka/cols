# 地图快速验收

已准备地图采用后台批量模拟和原生短局两类检查。后台检查每次改动都可运行；新地图第一次接入必须补做原生加载、移动与结算实采。共享回放、名片、技能HUD和动作的完整体验按专项复核，不要求每张图都重新冷启动并跑十分钟长局。小时级稳定性和完整视觉／听感仍独立验收。

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

诊断文件为1000分／60000ms。使用独立服务资料副本启动一次研究客户端，harness必须显式指定诊断catalog及`--service-data-dir`，不能使用正常`client-work/server-data`。原生批量工具不负责停止或重启客户端。

已有合适的运行实验时：

```sh
python3 tools/verify_native_maps.py \
  --catalog research-output/map-smoke-new/catalog.json \
  --experiment research-output/gamehub/client-EXPERIMENT/experiment.json \
  --maps mp_bog_sh mp_crash_tropical mp_killhouse_sh \
  --output research-output/native-map-smoke-new.json
```

`--maps`按给定顺序运行；未列地图保持未检查。原生创建房间、箭头选图、准备、有限移动和结算返回只使用正常系统输入。复用同一客户端、同一房间，不自动截图。结算按钮在state0后稍晚才接受点击；保留的12行统计不是当前页面标记，工具等待切换后返回，再核对箭头变化。

只读`--map-check`探针限定读取本图身份、12名角色的现有组合模型／动画树及本图小地图贴图；不遍历全部资源、不调用客户端函数。坠机使用真实`compass_map_mp_crash_dlc`。批量工具检查本图CRC、同一条命的玩家移动、人物资源和非默认贴图，并连续采样原生游戏／回放状态。输入前重新取可玩状态，W／D／S每次最多0.7秒，至少一次同生命周期移动8单位；不能把重生传送当成移动。结算必须是本图ID、12名参与者、单个本地玩家及非零击杀。读数切换期间最多重试三次；仍失败则保存失败报告，不标为通过。

报告保存原始样本路径、普通游戏／回放／中场帧数和AAR。回放、前三名画面及声音的完整观感不是本工具的通过条件。选择循环、初始化失败、缺少模型或默认小地图都会中止本次原生批次，需先定位失败；不反复盲目重启整个目录。

快速验收实验、正常账号恢复及剩余边界见`MAP_RESOURCE_PREPARATION.md`和`REVIEW_PENDING.md`。

`research-output/quick-native-podium-two-map-proof.json`记录`client-1791774867798`同一客户端／房间的Bog→坠机两局，合计271.91秒。Bog155.37秒含创建／选图，坠机116.54秒；两图均读到12名角色、非默认1024贴图、同生命周期移动112.62／133.37单位及12行结算。最终回放与PM_INTERMISSION6有原生采样，排名场景打开／ACK／关闭和结算有服务日志；两局AAR胜负／比分与服务端逐项一致。工具没有截图，不将这些状态样本当作完整演出验收。

完整检查`fast-map-podium-final-check.log`通过444项Rust、61项Python及fmt／Clippy；优化构建见`fast-map-podium-release.log`。隔离诊断结束时证书已恢复，正常账号文件逐项哈希不变，见`fast-map-account-restoration-proof.json`。Afghan和安全屋未包含在这次原生两图批次中；四图后台检查及此前各图实机证据分别保留。
