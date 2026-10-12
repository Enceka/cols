# 团队竞技地图覆盖

客户端原生`mp/coMapList.csv`有26条`pvp`且模式包含`war`的记录。`map-registry-3.3.3.3.json`保存其真实ID、模式、三个原始标志、文字键及角色规则；`prepare_map_registry.py`验证捕获身份、表布局和名称／ID唯一性。运行时准备文件与结算使用该表身份，未知地图及错误文件仍拒绝；元数据不代表资源已准备。

当前正常catalog包含阿富汗、安全屋、坠机之地、Bog、交叉火力、庄园、贫民窟、山景别墅和杂草丛生，均为7500分／600000ms。`prepare_tdm_map.py`接受本版26条身份，随后仍要求本图已初始化CRC、实体、碰撞、武器、角色及小地图绑定，未使用其它地图CRC或默认图片补齐。

只读IFS文件列表盘点：362个包、324个可识别列表、73839个观察路径。下表23张图观察到中文主图／load／const路径；三张没有观察到，不据此断言包坏损或资源确实缺失。列表是观察并集，没有验证最终patch覆盖视图、压缩块完整性或依赖闭包。证据为`research-output/full-map-resource-list-audit.json`及`full-map-coverage-audit.json`。

| 原生ID | map | 准备状态 | 观察到中文主图／load／const |
| --- | --- | --- | --- |
| 1 | mp_afghan | 已准备 | 有 |
| 2 | mp_bloc_sh | 待准备 | 未观察到 |
| 3 | mp_bog_sh | 已准备 | 有 |
| 4 | mp_crash_tropical | 已准备 | 有 |
| 5 | mp_cross_fire | 原生通过 | 有 |
| 6 | mp_estate_tropical | 原生通过 | 有 |
| 7 | mp_fav_tropical | 原生通过 | 有 |
| 8 | mp_killhouse_sh | 已准备 | 有 |
| 9 | mp_nuked | 候选出生校验失败 | 有 |
| 10 | mp_overgrown | 原生通过 | 有 |
| 11 | mp_rust_long | 待准备 | 有 |
| 13 | mp_storm_spring | 待准备 | 有 |
| 14 | mp_ambush_sh | 待准备 | 有 |
| 15 | mp_firingrange | 待准备 | 有 |
| 16 | mp_cargoship | 候选120秒无交战击杀 | 有 |
| 50 | mp_highrise | 待准备 | 有 |
| 53 | mp_dr_box | 待准备 | 有 |
| 56 | mp_dr_cistern | 待准备 | 有 |
| 60 | mp_abandon_sh_mk | 待准备 | 有 |
| 63 | mp_seatown | 待准备 | 有 |
| 67 | mp_utopia | 待准备 | 未观察到 |
| 68 | mp_roundabout | 待准备 | 未观察到 |
| 71 | mp_mountain | 原生通过 | 有 |
| 79 | mp_terminal_cls | 待准备 | 有 |
| 83 | za_coliseum | 待准备 | 有 |
| 84 | mp_subbase_sh | 待准备 | 有 |

26张身份的任务配置一次生成。`materialize_map_batch.py`先后台检查已有准备并隔离坏候选，再自动补齐有观察路径的待准备图；全部资源准备尝试结束后批量后台验收，再使用同一客户端进行原生短局。单图失败保存状态／日志，隔离该候选或诊断会话，继续后续图；正常目录只收录原生通过项。已有完整采集优先重新准备，只有缺少采集时才启动对应图资源实验。`mp_bloc_sh`、`mp_utopia`、`mp_roundabout`需要继续核对未识别列表和实际patch视图；三个原始标志的完整UI语义尚未核实。

通用采集入口、小地图材质证明、地图模型编号及玩家碰撞掩码导航已接入，mp_bog_sh 的12人对局和Bot交战已实采。其余地图逐图恢复；剧情／合作／其它模式和连杀不在当前团队竞技统计内，全地图尚未完成。详见 `MAP_RESOURCE_PREPARATION.md`。

验收使用[快速批量流程](FAST_MAP_ACCEPTANCE.md)：九张候选后台通过；四图早期实测含资源加载约2～5.4秒。新图原生短局使用隔离资料和同一客户端／房间，减少重复冷启动。共享表现和长时间稳定性另做专项复核，后台通过不替代原生通过。
