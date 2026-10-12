# 原生地图入口与容量

本版前端的`scaleform/default/codo.gfx`通过只读探针导出后在进程外解析；原始客户端、资源和界面脚本保持完整。源码证据保留于`remaining-map-analysis/frontend-source`，捕获SHA及字段依据见`native-map-entry-proof.json`。命令行静态导出依据[JPEXS官方说明](https://github.com/jindrapetrik/jpexs-decompiler/wiki/Commandline-arguments)，不执行界面脚本或调用客户端函数。

`codo.constants.table.Map`将第10／11列定义为最小／最大人数，第35列为`NO_CUSTOM_ROOM`。`MatchService`按`build_version_column`选取前三列之一，仅值1的行进入地图列表。当前客户端只读实测该列为2；`mp_bloc_sh`、`mp_utopia`、`mp_roundabout`在此列均为0，且当前资源列表没有观察到各自主图／load／const路径。这三条身份继续保留，但不能通过更改客户端版本列或其它地图资源将其标记为可玩。

`LobbyController.refreshRoomMaps`在自建房同时检查本房间人数上限、当前模式以及`noCustomRoom=false`。`mp_dr_box`、`mp_dr_cistern`的第35列为1，不能从自建房箭头进入；其原生路径是快速赛单房间。`za_coliseum`最大人数4，也不能通过12人房间的箭头进入。其实际入口和模式还需独立恢复，不把元数据中的`war`字段当作已实现4人玩法。

`--resource-capture-playlist`为显式单图采集绑定原生快速赛单。要求固定客户端build、已预订的publisher版本、指定赛单及首条地图序号；原始阶段无本图初始化资料时只保持传输，已有资料时只发送本图元数据，均不发布玩法快照或回退其它地图CRC。黑房批量采集会在独立服务目录中将首个人机赛单绑定为本图，并通过原生快速匹配／准备按钮进入；不反复尝试被禁止的自建房箭头，不写正常publisher。

两个黑房均已进入正常目录，正常赛单版本8提供“黑房人机”下的两个独立入口。各图保持7500分／600000ms。它们使用各自实体、BSP世界碰撞、武器、角色、出生点及真实1024小地图材质；共同NCS CRC4219922283不豁免地图身份检查。方箱保留23／23开始点，蓄水池保留12／12开始点；60秒后台模拟分别记录11个Bot移动、75／74次击杀、72／70次重生和排名场景通过，见`blackroom-server-acceptance/server.json`。

`blackroom-native-acceptance`的原生短局分别为96.79／86.53秒，确认12名角色／动画树、本图1024贴图、同生命周期移动137.11／148.40单位及12行结算，AAR600:1000／1000:900与服务端一致，全程0截图。快速房间使用原生投票界面，每图使用独立客户端；没有将缺少自建房箭头作为失败，也没有宣称快速房间结算后复用已验证。证据及正常准入见`blackroom-admission/admission.json`。

`blackroom-compass-capture-retry/result.json`还保留日志输出误用`proof`键的异常；准备文件实际包含`material_proof`并通过材质及后台检查。该异常不能被删去或当作整次自动流程通过，后续独立报告核对资源与玩法结果。
