# 原生小地图资源请求

高楼和潜艇基地的原始初始化采集仅看到4×4默认小地图材质，资源列表却有本图贴图路径。`--resource-compass-probe`让客户端通过正常gamestate的CS2037／2038请求本图候选材质，随后只读采集真实材质及贴图；不调用客户端函数、不改内存、不生成或替换客户端资源。

该选项必须与`--resource-capture-map`使用。当前接受`mp_highrise`、`mp_subbase_sh`、`mp_dr_box`、`mp_dr_cistern`和`za_coliseum`；候选JSON核对固定客户端SHA、地图、CRC、实体捕获SHA格式、本图`compass_map_<map>`名称和有效投影角点。角点由本图捕获实体的`northyaw`及两个`minimap_corner`计算。服务器还核对初始化元数据的地图／CRC，并拒绝带出生位置、武器、碰撞、Bot或其它玩法世界的准备文件。该路径无玩法快照，`resource-capture.json`明确记录`compass_candidate_requested`；候选不包含材质证明，不能作为正式游戏准备。

`materialize_map_batch.py`对上述五图的新初始化自动请求候选；完整旧缓存缺少真实材质时，仅启动一次资源初始化补采`characters`，保留实体、武器、角色服饰及碰撞。捕获最多重试三次，每次仍使用正式`minimap()`的本图SHA／CRC和非默认真实尺寸检查。旧碰撞缺少BSP世界证明时随后独立补采碰撞；全部准备仍按本图胶囊筛选出生点、后台检查及原生短局验收。

`compass-candidate-live/result.json`确认两图均读到本图1024×1024、`render_default=false`材质。高楼CRC4261926685及潜艇基地CRC3503358511来自各图NCS，投影角点来自各自捕获实体；原始缺材质样本保留。诊断结束后正常资料与准备哈希不变、库存6772及证书恢复均确认。这个资源结果不替代加载、移动、人物、交战或结算验收。

边界回归确认候选无法通过正常小地图材质验证、不同地图／CRC及其它地图材质被拒绝；准备含玩法世界时拒绝资源探测。Python采集入口拒绝将候选用于原生玩法或正常恢复会话。检查见`compass-candidate-final-check.log`与`map-compass-starts-complete-check.log`。

竞技场候选请求已读到本图512×512非默认贴图；CRC2197734643及实体角点独立核对，四人原生玩法通过，见`arena-admission/admission.json`。
