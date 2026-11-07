# Infinite Mosaic Gallery

基于 **Rust + GPUI Kit** 的原生沉浸式本地图片墙。以深黑画布、直角几何磁贴和自由平移缩放为核心。

> 项目正在按阶段实现；详见 [功能路线图](docs/ROADMAP.md) 和 [设计规范](docs/DESIGN.md)。

## 技术

- Rust 2021
- [GPUI Kit](https://github.com/longbridge/gpui-kit) 0.9
- 原生文件夹选择器 `rfd`
- 自研确定性 Mosaic 几何布局和可见区域裁剪

## 快速开始

```bash
cargo run --release
# 也支持在启动时提供文件夹路径
cargo run --release -- /path/to/photos
```

需要 Rust stable、对应系统 GPUI Kit 构建依赖和图形环境。首次运行可点击「打开文件夹」浏览本地图片。

## 交互

- 鼠标左键拖动画布；滚轮以指针为锚点缩放
- 点击磁贴聚焦，Esc 关闭；←/→ 切换图片
- `+` / `-` 缩放、`0` 重置视角
- 进入文件夹后后台枚举图片，当前版本默认只读取当前文件夹，支持递归选项

## 已知边界

早期版本使用 GPUI 的原生图片加载和可见区域裁剪，尚未完成有界缩略图 GPU 缓存、真实磁贴弹簧让位、文件变动监听及发布级性能验收。不要把 MVP 当作已通过大型图库的生产版本。

项目借鉴 [Peach Video Wall](https://github.com/peach-mitao/peach) 和 [Folia PosterWall](https://github.com/chthollyphile/folia-major) 的视觉语言，但几何模板和实现为本项目原创，没有直接复制 AGPL 代码。

## 许可证

本项目尚未选定代码许可证；引入任何上游源码前须复核许可及署名义务。
