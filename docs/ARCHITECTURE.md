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
        └── read-only folder enumeration → sorted photo paths
```

## 模块边界

| 文件 | 责任 |
| --- | --- |
| `src/main.rs` | GPUI Kit 初始化、Window 建立、可选 CLI 文件夹 |
| `src/app.rs` | GPUI View / 输入事件 / 状态 / 后台结果合并 |
| `src/catalog.rs` | 照片格式判断、目录枚举、排序 |
| `src/camera.rs` | 世界坐标与视口变换、缩放与平移 |
| `src/layout.rs` | 原创不重叠布局模板、只生成可见磁贴 |

## 读写策略

- 用户图片：原始路径只读，程序不会删除/修改/重命名。
- v0.1：不建立数据库，不创建用户数据和缓存目录。
- 后续：应用数据路径下保存索引与缩略图，明确缓存配额及清理策略。

## 渲染原则

v0.1 使用 GPUI 原生 `img(Path)` 加载图片，结合几何视口裁剪减少同时存在的元素数。需要注意 GPUI 内部图片缓存不会因为元素被视口裁掉就自动保证 GPU/CPU 内存上限；这不等价于完整的有界缩略图缓存。下一阶段以主动维护缩略图多级缓存替换直接读取原图。

## 异步正确性

用户每次打开或刷新文件夹都增加 generation。后台文件枚举完成后，只有 generation 匹配的结果才能覆盖当前图库。每次新的请求会替换旧前台 Task 句柄。

## 上游和许可证

- [GPUI Kit](https://github.com/longbridge/gpui-kit)：Apache-2.0
- [Peach](https://github.com/peach-mitao/peach)：视觉参考；上游项目 AGPL
- [Folia](https://github.com/chthollyphile/folia-major)：布局思想参考；上游项目 AGPL

本仓库的 12×8 模板独立设计，不复制 Folia 的模板数组或实现代码。
