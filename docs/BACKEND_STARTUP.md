# 地图加载与客户端启动

`gamehub_run.py`在启动Wine客户端之前确认观察服务、Rust AUTH及游戏服务就绪。AUTH捕获manifest和已绑定端口不能证明地图加载完成：准备文件的碰撞、导航与武器资料加载仍可能阻塞后端处理认证。游戏实验额外等待`tls-status.log`中的`experimental Rust game UDP endpoint ready`，该消息在全部准备地图加载后产生；不通过额外连接或握手探测端口。

游戏服务就绪等待上限240秒，AUTH单独实验保持5秒；任一自有后端退出立即报错。manifest记录`backend_ready_wait_seconds`。`Lab.start`用300秒界限等待真正启动的客户端manifest，随后才进入原生前端／库存等待。正常24份战术／冲锋目录的完整加载实测204709ms，见 `charge-mode-20261015/normal-catalog-server-acceptance.json`。超时、SIGTERM及其它异常仍进入自有进程和临时证书清理。

`highrise-server-startup-recovery`的正常恢复实验等待后端30.158秒后启动客户端，确认前端及6772库存。普通服务资料和资源恢复记录保留在`normal-restoration.json`；重新登录产生的私有档案上传列在`normal_restore_changed_files`，不混同为测试期间对正常资料的写入。启动条件、后端退出及有界超时有独立工具回归。
