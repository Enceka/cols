# 一键准备本地资源

资源准备入口位于管理面板的“开服设置”。它将用户提供的本地资料整理成独立运行目录，校验成功后自动填入默认地图和地图目录文件，再点击“一键开服”。仓库与工具不附带、下载或发布原版地图、客户端、密钥与素材。

## 在面板操作

1. 在 `server/` 下运行 `target/release/codol-server admin`，打开 `http://127.0.0.1:39080/#hosting`。
2. 停止当前受管服务，保存尚未保存的开服设置。
3. 在“本地资源准备”选择来源，输入服务器本机的绝对来源路径和输出父目录。
4. 点击“准备并应用资源”。进度显示在本页，具体错误见“运行日志”；取消使用“取消准备”。
5. 完成后检查自动填入的地图路径和服务资料目录，再点击“一键开服”。工具不会自动开启服务。

| 来源 | 输入 | 运行要求 |
| --- | --- | --- |
| 已有地图目录 | `catalog.json`、已有地图目录 JSON，或包含它的目录 | Rust 服务程序；可用于 macOS／Linux |
| 已有只读采集 | `capture-plan.json`，或包含 `assets.json` 与 `mp_地图名-resource-paths.json` 的目录 | Python 3.9或更高、随源码提供的 `tools/` 与 `fixtures/`、服务程序 |
| 本机客户端 | 用户自己的固定 3.3.3.3 完整客户端目录（真实文件，不含软链接） | macOS、已配置的项目 GameHub Wine10 环境、Python 3.9或更高、Xcode 命令行工具、足够的本地空间（同卷优先使用 APFS 克隆） |

已有地图目录会复制其中全部条目；模式与地图筛选只用于采集转换。已有采集可选择 `war`（战术）或 `cha`（冲锋）；地图留空表示转换所选模式的全部条目。客户端自动采集只支持战术，留空时采集 `mp_afghan`，可填入逗号分隔的已登记原生地图名。地图存在于登记表不表示它已完成资源或玩法验收。

客户端自动采集会创建独立客户端与 Wine 环境副本、独立登录身份及临时服务资料，生成并使用本地连接证书，通过正常客户端界面加载地图，读取资源定义、实体、初始化常量、世界碰撞、武器和角色。它会切换游戏前台并发送普通鼠标输入，运行时应关闭原游戏并暂停其它操作。不会终止其它客户端。AUTH、大厅、游戏和资源观察器自动选择空闲回环端口；STUN沿用本版已验证的3074／3078。默认独立运行发现服务，需要这两个端口空闲；勾选“共用已有本机发现服务”或使用 `--reuse-stun` 时，只在协议校验通过后使用已有无状态发现服务，其它后端与账号仍独立。单图自动流程已以阿富汗验证至原生初始化、资源导出和独立运行包；完整玩法、视听及多图连续采集仍分别验收。

## 已有目录的命令行方式

```sh
target/release/codol-server prepare-resources \
  --source /absolute/path/to/catalog.json \
  --output /absolute/path/to/new-resources \
  --report /absolute/path/to/new-result.json
```

`--source` 为目录时识别 `catalog.json`、`native-tdm-map-catalog.json` 或 `native-three-map-catalog.json`。输出目录和报告必须为新路径。工具按内容散列去重全部运行时引用，改写为相对路径，并使用真实服务端加载器核对地图／模式、CRC 和资源依赖。现有资源包带 `resources.json` 时，还会核对其文件散列，拒绝损坏文件。

输出结构：

```text
new-resources/
  catalog.json
  resources.json
  maps/
  assets/
```

该目录可整体迁移，服务端的 `--game-map-catalog` 指向 `catalog.json`，`--game-state` 使用报告中的默认地图路径。`resources.json` 记录资源散列和校验范围，不表示原生画面、听感或完整玩法验收通过。

## 从保留采集转换

```sh
python3 tools/prepare_resources.py captures \
  --source /absolute/path/to/capture-plan.json \
  --output /absolute/path/to/new-preparation \
  --server /absolute/path/to/codol-server \
  --mode war --maps mp_nuked
```

推荐的采集计划如下。路径相对于计划文件，也可以使用绝对路径；所有内容由用户在本机提供。

```json
{
  "schema_version": 1,
  "client_sha256": "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
  "assets": "assets.json",
  "maps": [{
    "map": "mp_nuked",
    "mode": "war",
    "captures": {
      "net-constants": "mp_nuked/constants.json",
      "entities": "mp_nuked/entities.json",
      "collision": "mp_nuked/collision.json",
      "arsenal": "mp_nuked/arsenal.json",
      "characters": "mp_nuked/characters.json",
      "outfits": "mp_nuked/outfits.json"
    }
  }]
}
```

六类地图采集均为必需；全局 `assets` 来自只读 `--assets` 导出。碰撞必须含当前 BSP 世界归属证明 `world_membership.bsp_root = 0`；旧采集缺少证明时需按[地图资源流程](MAP_RESOURCE_PREPARATION.md)补采。初始化 CRC、出生点、小地图材质及角色绑定仍须通过既有转换器的检查，不会用默认素材或其它图的数据补齐。

可选 `voice`、`props`、`rewards` 分别接入已有语音、场景物件与雷达转换器；缺少它们时不会自动生成对应功能资源。客户端初始化采集只生成基础地图资源，语音转换器要求正式对局状态，需使用相应保留采集另行导入。既有地图目录导入会保留全部受支持引用，包括核弹资源。转换完成的运行目录是 `new-preparation/runtime/`，其它目录用于本地采集、准备和诊断，不应作为服务器资源包分发。

## 从本机客户端采集

```sh
python3 tools/prepare_resources.py client \
  --source /absolute/path/to/CODOL \
  --output /absolute/path/to/new-preparation \
  --server /absolute/path/to/codol-server \
  --maps mp_afghan
```

原生客户端来源须为完整文件副本。工具会拒绝软链接，避免客户端通过链接访问或改动来源外的文件；启动IFS另按固定完整哈希重建到隔离副本。已有环境使用恢复文件链接时，先在独立来源副本中放入可正常读取、哈希匹配的真实文件，参见[启动与文件访问恢复](CLIENT_STARTUP_RECOVERY.md)。

独立命令可通过 `--certificate-dir` 指定含 `server.pem`／`key.pem` 的本地证书目录；省略时使用本机 OpenSSL 生成。面板使用它管理的配套证书。客户端与 Wine 副本位于项目忽略的 `client-work/resource-*` 目录，账号、原始采集及日志位于本次输出目录。它们可能含本地账号资料，不能与最终 `runtime/` 混合。结束后检查 `capture-cleanup.json`；不再需要诊断时，可在确认本次进程全部结束后自行删除这些副本与中间文件。

## 失败与校验范围

失败或取消不会应用半成品，不改变原地图配置、玩家资料和端口设置。目录导入使用临时目录完成校验后再发布；转换／采集可能保留私有诊断文件，下次选择新输出路径。面板任务可通过停止按钮取消；关闭浏览器不取消任务，退出面板进程会通知本次工具退出。系统文件I/O不可中断时，面板会报告进程尚未回收，不将其标为成功；文件访问恢复见[启动与恢复](CLIENT_STARTUP_RECOVERY.md)。

自动回归使用合成资源检查复制、去重、引用重写、迁移、损坏拒绝、取消清理和面板配置应用。核弹镇保留采集已通过离线转换与迁移，阿富汗已通过完整自动采集、基础运行包校验和60秒后台对局检查，覆盖Bot移动、击杀、重生与前三名场景。合成回归和资源阶段的原生初始化均不替代正式对局、实际画面与听感检查。

## 资源来源与分发

本项目选择用户自行提供与采集的方式。将原版数据转成 JSON、碰撞或服务器格式，不会自动取得复制和分发许可；本地持有或可读取也不等于具有全部使用权。范围遵循 [LEGAL.md](../LEGAL.md)，对外提供任何原始或派生游戏数据前应另行确认授权。

AlphaBuild 的打包工具将程序与原版 `game/` 数据分开：后者装入原有 FF／IWD 和从客户端导出的密钥，并要求保持私有；不是开放地图下载源。它针对 1.1.1.13，本项目针对 3.3.3.3 的 IFS，不能直接复用其地图读取链。
