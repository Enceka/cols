# 协议样本与本版元数据

本目录保存独立合成向量、脱敏公开握手及本版字段／必要资源标识。目标程序摘要见[客户端基线](../../docs/CLIENT_BASELINE.md)。不包含真实账号、会话密钥、有效票据、私钥、完整游戏资源或原始业务明文。

`synthetic`仅验证结构与编码，`sanitized`保留已观察结构并替换身份；字段／资源元数据证明本版消费者和数据来源，不自动证明玩法完成。二进制的来源和摘要由同名JSON说明。`evidence_only`及`gameplay_verified`的整体标志须结合[功能状态](../../docs/REFERENCE_ALIGNMENT.md)和各奖励[覆盖表](../../docs/KILLSTREAK_COVERAGE.md)，不能扩大或否定单项验收。

公开档案合成向量不是可用玩家种子，地图／模式条目不是资源准备，参考票据向量不是腾讯认证。原始采集和完整资源均在本机忽略目录。

## 认证、大厅与公共文件

- `auth-task10-sanitized.json`
- `auth-ticket-alpha-vector.json`
- `chat-3.3.3.3.json`
- `clienthello-3.3.3.3.bin`
- `clienthello-3.3.3.3.json`
- `lobby-clienthello-3.3.3.3.bin`
- `lobby-clienthello-3.3.3.3.json`
- `lobby210-synthetic.json`
- `publisher3333.json`

## 档案、配装、房间与结算

- `backpack-class-offsets-3.3.3.3.json`
- `character-slots-3.3.3.3.json`
- `player-card-3.3.3.3.json`
- `player-inventory-synthetic.json`
- `private-profile-synthetic.bin`
- `private-profile-synthetic.json`
- `public-profile-synthetic.bin`
- `public-profile-synthetic.json`
- `room-browser-synthetic.json`
- `room-create-synthetic.bin`
- `room-loadout-synthetic.json`
- `room-map-capabilities-3.3.3.3.json`
- `room-roster-synthetic.bin`
- `scoreboard-playerdata-3.3.3.3.json`
- `summary-playerdata-3.3.3.3.json`

## 地图与模式

- `charge-map-labels-3.3.3.3.json`
- `charge-map-registry-3.3.3.3.json`
- `charge-spawn-coverage-3.3.3.3.json`
- `map-registry-3.3.3.3.json`
- `team-modes-3.3.3.3.json`

## 快照、武器、移动与战斗

- `bullet-impact-3.3.3.3.json`
- `corpse-entity-fields-3.3.3.3.json`
- `damage-feedback-resources-3.3.3.3.json`
- `death-waypoint-hud-3.3.3.3.json`
- `dive-3.3.3.3.json`
- `enemy-snapshot-fields-3.3.3.3.json`
- `entity-state-fields-3.3.3.3.json`
- `held-weapon-3.3.3.3.json`
- `hit-feedback-hud-3.3.3.3.json`
- `native-event-names-3.3.3.3.json`
- `obituary-3.3.3.3.json`
- `player-state-fields-3.3.3.3.json`
- `weapon-drops-3.3.3.3.json`

## 技能、投掷物与语音

- `emp-status-3.3.3.3.json`
- `gas-status-3.3.3.3.json`
- `passive-skills-3.3.3.3.json`
- `progression-3.3.3.3.json`
- `scavenger-3.3.3.3.json`
- `tactical-status-3.3.3.3.json`
- `throwables-3.3.3.3.json`
- `voice-3.3.3.3.json`

## 连杀、选点与载具

- `aircraft-audio-3.3.3.3.json`
- `aircraft-bindings-afghan-3.3.3.3.json`
- `helicopter-fields-3.3.3.3.json`
- `killstreaks-3.3.3.3.json`
- `location-selection-3.3.3.3.json`
- `missile-fields-3.3.3.3.json`
- `remote-links-3.3.3.3.json`
- `remote-missile-afghan-3.3.3.3.json`
- `turret-fields-3.3.3.3.json`
- `turret-state-3.3.3.3.json`
- `vehicle-animation-3.3.3.3.json`

格式与行为的详细说明统一见[文档索引](../../docs/README.md)。新增样本应记录方向、来源摘要、字段依据及证据类型，不将普通roundtrip当作独立向量或原生验收。
