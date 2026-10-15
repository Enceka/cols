# 复用 AlphaBuild 逻辑，保持纯 Rust

## 可复用性结论

参考实现分离内部状态与客户端格式，可借鉴模块边界。Rust认证、大厅、DATA加密、NCS CRC、快照字段表和输入解码保持本版已验证协议，玩法按对应边界适配。

本地参考README指定1.1.1.13。2.6.9.7加载文档只提供资源分析线索，不能据此推断3.3.3.3的服务端适配范围。

| 可沿用的边界/逻辑 | 本地参考源码 | 本版落实方式 |
| --- | --- | --- |
| 输入排队、时钟归属、每条命令执行 | `src/codol/player/codol_pmove_c1.h`、`src/codol/net/codol_usercmd_queue.*` | Rust输入队列与玩家状态拥有commandTime；快照时钟保持独立 |
| 玩法状态构造与线格式分离 | `src/codol/snapshot/codol_frame_builder.*`、`codol_engine_state.*` | 先构造Rust语义状态，再按3.3字段表编码；不直接复制C1字段向量 |
| 武器默认装配、附件合法性与模型映射 | `src/codol/weapon/codol_weapon_compose.*`、`src/codol/player/codol_bot_loadout.c` | 按本版实际资源表生成完整装配，不手选一组任意零件 |
| 射击、换弹、ADS、武器切换 | `src/codol/weapon/` | 覆盖完整状态转换、计时、弹药守恒及事件；不是每帧重新填初始值 |
| 位移、地面、滑动、阶梯、姿态 | `src/codol/player/codol_pmove_c1*` | Rust移动核加明确的trace接口，逐项核对3.3的字段、标志和参数 |
| 碰撞、地图、脚本和AI | `src/codol/collision/`、`assets/`、`script/` | 独立资源与玩法模块，按实际状态和本版消费者适配 |

上表说明参考关系，不声明全部适配已完成。若直接改写参考代码，须记录来源并保留其许可要求。当前能力按[功能状态](REFERENCE_ALIGNMENT.md)逐项确认。

## 已证实的适配工作

- 参考玩家字段表为244项，3.3静态提取为235项；例如C1当前武器offset0x33c，本版0x340。字段数量、偏移、标志和数组段必须由本版编解码适配。
- 参考武器资源固定部分为0xbfc字节，本版0x42e3b0复制0x38a个word，即0xe28字节。资源加载器和字段偏移不能原样沿用。
- 组合武器名格式也不同：参考解析item/variant成对参数，本版解析 `** base flag extra attachmentId...`，配件名称注册表与属性表再分开解析。
- 游戏DATA与大厅加密差异已有实测，见GAME_TRANSPORT_PROGRESS和AUTH_LOBBY_PROGRESS。不能以“改协议/版本常量”替代完整边界适配。
- 参考构建依赖32位x86工具链与保留汇编，附带exe为PE32。当前Mac PATH未发现其要求的NASM/交叉gcc；纯Rust路线无需为了链接该核心改变现有运行架构。

## 当前适配范围

装备、移动、射击、Bot、对局、回放与共享多人已有 Rust 实现；连杀按公共库存和效果控制器组织，剧情仅完成资源初始化与采集。模块结构见[服务端结构](SERVER_MODULARIZATION_PLAN.md)，实现及原生验证范围见[功能状态](REFERENCE_ALIGNMENT.md)。参考代码的可复用性不代表本版全部规则和表现已验收。
