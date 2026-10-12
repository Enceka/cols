# 团队竞技地图覆盖

客户端原生`mp/coMapList.csv`有26条`pvp`且模式包含`war`的记录。`map-registry-3.3.3.3.json`保存其真实ID、模式、三个原始标志、文字键及角色规则；`prepare_map_registry.py`验证捕获身份、表布局和名称／ID唯一性。运行时准备文件与结算使用该表身份，未知地图及错误文件仍拒绝；元数据不代表资源已准备。

当前正常catalog包含阿富汗、安全屋、坠机之地、mp_bog_sh，均为7500分／600000ms。`prepare_tdm_map.py`接受本版26条身份，随后仍要求本图已初始化CRC、实体、碰撞、武器、角色及小地图绑定，未使用其它地图CRC或默认图片补齐。

只读IFS文件列表盘点：362个包、324个可识别列表、73839个观察路径。下表23张图观察到中文主图／load／const路径；三张没有观察到，不据此断言包坏损或资源确实缺失。列表是观察并集，没有验证最终patch覆盖视图、压缩块完整性或依赖闭包。证据为`research-output/full-map-resource-list-audit.json`及`full-map-coverage-audit.json`。

| 原生ID | map | 准备状态 | 观察到中文主图／load／const |
| --- | --- | --- | --- |
| 1 | mp_afghan | 已准备 | 有 |
| 2 | mp_bloc_sh | 待准备 | 未观察到 |
| 3 | mp_bog_sh | 已准备 | 有 |
| 4 | mp_crash_tropical | 已准备 | 有 |
| 5 | mp_cross_fire | 待准备 | 有 |
| 6 | mp_estate_tropical | 待准备 | 有 |
| 7 | mp_fav_tropical | 待准备 | 有 |
| 8 | mp_killhouse_sh | 已准备 | 有 |
| 9 | mp_nuked | 待准备 | 有 |
| 10 | mp_overgrown | 待准备 | 有 |
| 11 | mp_rust_long | 待准备 | 有 |
| 13 | mp_storm_spring | 待准备 | 有 |
| 14 | mp_ambush_sh | 待准备 | 有 |
| 15 | mp_firingrange | 待准备 | 有 |
| 16 | mp_cargoship | 待准备 | 有 |
| 50 | mp_highrise | 待准备 | 有 |
| 53 | mp_dr_box | 待准备 | 有 |
| 56 | mp_dr_cistern | 待准备 | 有 |
| 60 | mp_abandon_sh_mk | 待准备 | 有 |
| 63 | mp_seatown | 待准备 | 有 |
| 67 | mp_utopia | 待准备 | 未观察到 |
| 68 | mp_roundabout | 待准备 | 未观察到 |
| 71 | mp_mountain | 待准备 | 有 |
| 79 | mp_terminal_cls | 待准备 | 有 |
| 83 | za_coliseum | 待准备 | 有 |
| 84 | mp_subbase_sh | 待准备 | 有 |

准备顺序：先覆盖有原生路径的团队竞技地图，逐张采集本版CRC、实体／碰撞、武器／人物模型并核验小地图材质；补齐本图出生及角色规则后加入catalog，再做原生选图、对局与结算实采。`mp_bloc_sh`、`mp_utopia`、`mp_roundabout`需要继续核对未识别列表和实际patch视图；三个原始标志的完整UI语义尚未核实。

通用采集入口、小地图材质证明、地图模型编号及玩家碰撞掩码导航已接入，mp_bog_sh 的12人对局和Bot交战已实采。其余地图逐图恢复；剧情／合作／其它模式和连杀不在当前团队竞技统计内，全地图尚未完成。详见 `MAP_RESOURCE_PREPARATION.md`。
