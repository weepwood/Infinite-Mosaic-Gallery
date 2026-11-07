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
        ├── read-only folder enumeration → sorted photo paths
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
| `src/catalog.rs` | 照片格式判断、目录枚举、排序 |
| `src/camera.rs` | 世界坐标与视口变换、缩放与平移 |
| `src/layout.rs` | 原创不重叠布局模板、只生成可见磁贴 |
| `src/thumbnails.rs` | 限额解码、EXIF 方向、768px PNG、内容版本键、原子写入及周期清理 |
| `src/image_cache.rs` | GPUI ImageCache LRU（128 个条目，淘汰时释放 GPU 纹理） |

## 读写策略

- 用户图片：原始路径只读，程序不会删除/修改/重命名。
- v0.2：不建立数据库；仅在系统应用缓存目录创建缩略图。
- 后续：应用数据路径下保存索引与缩略图，明确缓存配额及清理策略。

## 渲染原则

v0.2 的磁贴不再读取原图，而是使用 3 个后台并发任务生成 768px PNG 缩略图并按需展示；聚焦查看才读取原图。每个画布的图片使用 GPUI 有界 ImageCache，不使用 App 默认永久解码缓存。缓存按 128 个条目限制而非严格字节数配额，原图聚焦时仍可能造成瞬时内存峰值；大型图库压力测试及更精确的字节预算是后续工作。磁盘清理按文件修改时间近似淘汰最旧的缓存图，每生成 32 张触发一次，因此 512 MiB 是近似预算，不是严格上限。

## 异步正确性

用户每次打开或刷新文件夹都增加 generation。后台文件枚举完成后，只有 generation 匹配的结果才能覆盖当前图库。每次新的请求会替换旧前台 Task 句柄。

## 上游和许可证

- [GPUI Kit](https://github.com/longbridge/gpui-kit)：Apache-2.0
- [Peach](https://github.com/peach-mitao/peach)：视觉参考；上游项目 AGPL
- [Folia](https://github.com/chthollyphile/folia-major)：布局思想参考；上游项目 AGPL

本仓库的 12×8 模板独立设计，不复制 Folia 的模板数组或实现代码。
