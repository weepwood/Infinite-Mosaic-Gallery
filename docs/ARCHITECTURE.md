# 架构说明

```text
native window / GPUI Kit
  ├── header controls / folder selector
  ├── mosaic viewport (GPUI hit-testing + GPU image rendering)
  │    ├── Camera: pan + anchor zoom
  │    ├── Layout: finite deterministic mosaic + viewport culling
  │    └── Focus: enlarged image and keyboard navigation
  └── status footer
        ↑
  BackgroundExecutor
        ├── native filesystem events → bounded channel → 550ms batching → full rescan
        ├── read-only folder enumeration → persistent slot assignment
        └── priority preview generation (≤3 concurrent)
                    ↓
            versioned PNG disk cache (512 MiB trim)
                    ↓
          scoped recent-image cache (128 entries)
```

## 模块边界

| 文件 | 责任 |
| --- | --- |
| `src/main.rs` | GPUI Kit 初始化、Window 建立、可选 CLI 文件夹 |
| `src/app.rs` | GPUI View / 输入事件 / 状态 / 后台结果合并 |
| `src/catalog.rs` | 照片格式判断、后台目录枚举、文件大小与 mtime 版本、稳定空间排序 |
| `src/watcher.rs` | 原生文件监控、过滤应用缩略图与槽位索引事件、容量为 1 的通知队列 |
| `src/slots.rs` | 按文件夹的持久图片空间槽位、原子 JSON 索引、跨重启读取和校验 |
| `src/camera.rs` | 世界坐标与视口变换、缩放与平移 |
| `src/layout.rs` | 原创不重叠布局模板、只生成可见磁贴 |
| `src/reflow.rs` | 6×6 焦点区域、剩余区域几何分配、最短位移匹配与纯函数测试 |
| `src/focus_geometry.rs` | 原始磁贴至大图面板的插值矩形，视口裁切约束，纯函数几何测试 |
| `src/thumbnails.rs` | 限额解码、EXIF 方向、768px PNG、内容版本键、原子写入及周期清理 |
| `src/image_cache.rs` | GPUI ImageCache LRU（128 个条目，淘汰时释放 GPU 纹理） |

## 读写策略

- 用户图片：原始路径只读，程序不会删除/修改/重命名。
- v0.2：系统应用缓存目录存放缩略图，限额定期清理。
- v0.4：系统应用数据目录 `InfiniteMosaicGallery/slots-v1/` 存放相对路径至长期空间槽位的映射；保留被删除照片的路径作为墓碑（不重用槽位），文件夹根路径哈希后生成独立索引文件。
- 未来：精确 GPU 字节预算、重命名时继续跟踪文件身份，以及用户确认后清理长期墓碑。

## 渲染原则

v0.2 的磁贴不再读取原图，而是使用 3 个后台并发任务生成 768px PNG 缩略图并按需展示；聚焦查看才读取原图。每个画布的图片使用 GPUI 有界 ImageCache，不使用 App 默认永久解码缓存。缓存按 128 个条目限制而非严格字节数配额，原图聚焦时仍可能造成瞬时内存峰值；大型图库压力测试及更精确的字节预算是后续工作。磁盘清理按文件修改时间近似淘汰最旧的缓存图，每生成 32 张触发一次，因此 512 MiB 是近似预算，不是严格上限。

## 异步正确性

文件变动采用系统 `notify` 监听，用户可以关闭自动同步。事件进入容量为 1 的异步邮箱，并以 550ms 合并窗口触发后台全量重扫，防止批量复制产生无界事件队列。扫描完成后从持久索引恢复图片的空间槽位；删除文件不会使后续磁贴整体前移，新增文件追加到更高的槽位，同一路径重新出现时复用旧槽。仅当文件 size/mtime 不变时复用原缩略图与 GPU 图像条目。

用户每次打开或刷新文件夹都增加 generation。后台文件枚举完成后，只有 generation 匹配的结果才能覆盖当前图库。每次新的请求会替换旧前台 Task 句柄。手动刷新期间旧图库仍可显示，结果到达后通过路径重映射选中的照片；若图片被编辑，重新创建有界解码缓存，避免复用同路径的旧 GPU 内容。缓存文件的周期清理可能使内存中的预览路径失效，因此只在有限时间间隔检查当前视口预览路径并允许重新生成。

## 上游和许可证

- [GPUI Kit](https://github.com/longbridge/gpui-kit)：Apache-2.0
- [Peach](https://github.com/peach-mitao/peach)：视觉参考；上游项目 AGPL
- [Folia](https://github.com/chthollyphile/folia-major)：布局思想参考；上游项目 AGPL

本仓库的 12×8 模板独立设计，不复制 Folia 的模板数组或实现代码。

## 交互运动

选中图片时只对同一 12×8 几何块的磁贴计算新目标，其他块的目标矩形不变。GPUI Base spring 为每个图片 ID 的 x/y/width/height 建立稳定通道；取消聚焦时目标恢复原位置，Motion 自动连续收敛。弹簧不是帧驱动的数据模型：布局目标与图片身份始终是唯一事实来源。
