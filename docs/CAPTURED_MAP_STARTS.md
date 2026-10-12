# 捕获出生点的碰撞筛选

地图准备先保存`<map>-captured-starts.json`中的原始两队出生点，再通过`codol-server prepare-map-starts`使用本图真实碰撞筛选。保留点的坐标及顺序不变；不从其它模式借点、不复制或生成点。玩家与Bot初次出生和重生继续使用同一`grounded_spawn`站立胶囊贴地检查。

```sh
target/release/codol-server prepare-map-starts \
  --game-state research-output/MAP-captured-starts.json \
  --output research-output/MAP-state.json \
  --report research-output/MAP-start-selection.json
```

输出文件必须不存在。命令验证固定客户端SHA、原生TDM身份、已初始化CRC及碰撞地图／CRC，资源引用转为绝对路径。报告记录原始准备SHA、两队原始数量、保留的原始点，以及每个排除点的队伍、原索引、坐标和失败理由。任一队少于6个有效点时写失败报告且不生成正式准备。筛选后仍加载完整准备文件，验证资源、所有保留点及导航；其它错误保留诊断并返回非零。运行时逐点站立检查不变。

废弃都市`mp_abandon_sh_mk`的原始开始点为13／11个，Allies索引2的`[636.307,-1882.92,-62]`无法容纳站立胶囊。使用本图BSP世界碰撞筛选后保留13／10个原始点；`abandon-clear-starts-report.json`保留唯一排除项，`abandon-clear-starts-server-proof.json`的17图后台检查全部通过。原生短局195.47秒确认12人模型、同生命周期移动116.54单位、真实小地图及12行结算；胜负／比分与服务端一致，见`eighteen-map-admission.json`。

该筛选不证明所有地面、动态子模型或特殊地图规则已经完整恢复。新图仍需原生加载、移动和交战／结算验收；早期碰撞捕获的范围升级见`WORLD_COLLISION_SCOPE.md`。
