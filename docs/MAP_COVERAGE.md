# 团队竞技地图覆盖

客户端原生`mp/coMapList.csv`有26条`pvp`且模式包含`war`的记录。`map-registry-3.3.3.3.json`保存其真实ID、模式、三个原始标志、文字键及角色规则；`prepare_map_registry.py`验证捕获身份、表布局和名称／ID唯一性。运行时准备文件与结算使用该表身份，未知地图及错误文件仍拒绝；元数据不代表资源已准备。

当前正常catalog有22张原生通过图：阿富汗、安全屋、坠机之地、Bog、交叉火力、庄园、贫民窟、山景别墅、杂草丛生、锈迹之地、核弹镇、货轮、航站楼、暴风雨、伏击、靶场、废弃都市、滨海镇、潜艇基地、高楼及两个黑房，均为7500分／600000ms。黑房通过正常快速赛单进入，其余地图保留原生自建房路径。`prepare_tdm_map.py`接受本版26条身份，随后仍要求本图已初始化CRC、实体、碰撞、武器、角色及小地图绑定，未使用其它地图CRC或默认图片补齐。

只读IFS文件列表盘点：362个包、324个可识别列表、73839个观察路径。下表23张图观察到中文主图／load／const路径；三张没有观察到，不据此断言包坏损或资源确实缺失。列表是观察并集，没有验证最终patch覆盖视图、压缩块完整性或依赖闭包。证据为`research-output/full-map-resource-list-audit.json`及`full-map-coverage-audit.json`。

| 原生ID | map | 准备状态 | 观察到中文主图／load／const |
| --- | --- | --- | --- |
| 1 | mp_afghan | 已准备 | 有 |
| 2 | mp_bloc_sh | 当前版本列关闭，资源待核对 | 未观察到 |
| 3 | mp_bog_sh | 已准备 | 有 |
| 4 | mp_crash_tropical | 已准备 | 有 |
| 5 | mp_cross_fire | 原生通过 | 有 |
| 6 | mp_estate_tropical | 原生通过 | 有 |
| 7 | mp_fav_tropical | 原生通过 | 有 |
| 8 | mp_killhouse_sh | 已准备 | 有 |
| 9 | mp_nuked | 原生通过，世界碰撞已筛选 | 有 |
| 10 | mp_overgrown | 原生通过 | 有 |
| 11 | mp_rust_long | 原生通过 | 有 |
| 13 | mp_storm_spring | 原生通过，世界碰撞已筛选 | 有 |
| 14 | mp_ambush_sh | 原生通过，使用本图Convoy小地图材质 | 有 |
| 15 | mp_firingrange | 原生通过，世界碰撞已筛选 | 有 |
| 16 | mp_cargoship | 原生通过，世界碰撞已筛选 | 有 |
| 50 | mp_highrise | 原生通过，169块玻璃及单向落差路径已接 | 有 |
| 53 | mp_dr_box | 原生快速赛单通过，禁止自建房 | 有 |
| 56 | mp_dr_cistern | 原生快速赛单通过，禁止自建房 | 有 |
| 60 | mp_abandon_sh_mk | 原生通过，筛选后13／10个有效出生点 | 有 |
| 63 | mp_seatown | 原生通过，世界碰撞已筛选 | 有 |
| 67 | mp_utopia | 当前版本列关闭，资源待核对 | 未观察到 |
| 68 | mp_roundabout | 当前版本列关闭，资源待核对 | 未观察到 |
| 71 | mp_mountain | 原生通过 | 有 |
| 79 | mp_terminal_cls | 原生通过，世界碰撞已筛选 | 有 |
| 83 | za_coliseum | 最大4人，入口／模式待恢复 | 有 |
| 84 | mp_subbase_sh | 原生通过，真实小地图和世界碰撞已绑定 | 有 |

26张身份的任务配置一次生成。`materialize_map_batch.py`先后台检查已有准备并隔离坏候选，再自动补齐有观察路径的待准备图；全部资源准备尝试结束后批量后台验收，再使用同一客户端进行原生短局。单图失败保存状态／日志，隔离该候选或诊断会话，继续后续图；正常目录只收录原生通过项。已有完整采集优先重新准备，只有缺少采集时才启动对应图资源实验。`mp_bloc_sh`、`mp_utopia`、`mp_roundabout`需要继续核对未识别列表和实际patch视图；三个原始标志的完整UI语义尚未核实。

通用采集入口、小地图材质证明、地图模型编号及玩家碰撞掩码导航已接入，mp_bog_sh 的12人对局和Bot交战已实采。其余地图逐图恢复；剧情／合作／其它模式和连杀不在当前团队竞技统计内，全地图尚未完成。详见 `MAP_RESOURCE_PREPARATION.md`。

验收使用[快速批量流程](FAST_MAP_ACCEPTANCE.md)：正常22张图具有后台及原生短局证据；四图早期实测含资源加载约2～5.4秒。新图原生短局使用隔离资料并复用客户端，保留的房间验证实际箭头；原生退出频道后重建房间。暴风雨165.82秒，伏击→靶场同一客户端219.8／111.5秒，AAR与服务比分一致，见`room-storm-admission.json`及`room-sixteen-map-admission.json`。废弃都市195.47秒、滨海镇199.56秒，两图12人模型、小地图、移动和结算通过，AAR与服务端一致，见`eighteen-map-admission.json`；废弃都市出生点依据见`CAPTURED_MAP_STARTS.md`。潜艇基地231.86秒，移动135.31单位、12人模型／结算和本图小地图通过，AAR800:400与服务端一致，见`nineteen-map-admission.json`。小地图请求和高楼跨越缺口分别见`RESOURCE_COMPASS_REQUESTS.md`及`HIGHRISE_TRAVERSAL.md`。高楼原生短局162.22秒，移动142.52单位、12名角色／结算和真实小地图通过，AAR200:300与服务端一致，见`highrise-map-admission/admission.json`。其余4张仍需入口、容量或资源诊断，完整视觉和长时间稳定性另做专项复核。
