# EMP与硬链接

本版物品60006／`emp_grenade_mp`／武器74接入战术槽、Q阶段、真实数量、1000ms引信及碰撞飞行。原生爆炸资源为`explosions/emp_grenade`，使用EV_CUSTOM_EXPLODE93并携带真实开始时间，不设置烟雾持续位，不额外伪造EMP shock资源。

本版HUD operator154的分派调用`0x574130`，经本版CG指针读取PS+0x10、右移10位并检查bit0。该字段在0xc55f70表中为field54／28位，EMP掩码0x400与现有普通0x1000及回放0x800标志合并传输，不覆盖其它状态。`tools/prepare_emp.py`按固定客户端SHA核对实际分派及getter指令，`fixtures/protocol/emp-status-3.3.3.3.json`保留该代码哈希。

玩家期限采用Alpha/LAN中IW5参考移植的`_empgrenade.gsc`：生效10秒、短暂1000ms闪光Named预设、重复命中刷新期限，死亡、重生及结束清除。范围和普通1点伤害来自本版WeaponDef及现有墙体／角色暴露判定；队友免状态，投掷者自身可受影响。本版生产脚本和精确暴露几何未恢复。参考`_weapons.gsc`的8秒`emp_damage`用于可部署装备，当前没有将它当作玩家期限；部署装备消费者仍未实现。

硬链接60213／技能bit70在本版能力表中激活`modEmpProtectionScale=0`，免除EMP状态及附带闪光，普通1点范围伤害仍保留。未装备该能力时，表中默认0不能当作免疫：代码先检查实际激活，再读取倍率。玩家／Bot均按持有者的配置判定。Bot归档视图保留实际EMP位；原生回放字段、到期和移除的快照有回归。

安全屋隔离实采：`client-1791764868404`确认原生EMP特效、Q动作／飞行、生命99／100、Named index5／1000ms，67个命令稳定的EMP帧读取0x1400；同一条命到期恢复0x1000，后续400帧为100生命、无EMP位及零冲击期限。硬链接`client-1791765040781`确认原生技能bit70、同一EMP特效及普通1点伤害，全程无EMP位和闪光，后续400帧正常。见`research-output/emp-native-live-proof.json`。本轮无截图，不将状态字段和特效实例当作完整HUD／声音观感验收。客户端CG与权威时钟有渲染／快照延迟，原生采样记录窗口，10秒期限依据参考规则及状态机回归。

`emp-final-check.log`确认437项Rust、52项Python及fmt／Clippy通过，`emp-release.log`确认release构建通过。正常账号、库存、钱包使用独立副本隔离，见`emp-account-proof.json`。

待恢复／复核：本版生产公式、其它地图、普通／最终回放实机、完整HUD／声音／短暂闪光观感、遥控状态与特殊模式门槛、部署装备及全局EMP。硬链接的反侦察机、抗标记、干扰器免疫消费者尚未全部接入；本项只覆盖EMP。
