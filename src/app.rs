//! GPUI native presentation. Geometry, image catalog and camera math are testable
//! without instantiating a GPU window.
use std::path::PathBuf;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use crate::camera::Camera;
use crate::catalog::{self, Photo};
use crate::layout::{self, Rect};

const HEADER_HEIGHT: f32 = 68.0;
const FOOTER_HEIGHT: f32 = 36.0;
const BACKGROUND: u32 = 0x070707;
const SURFACE: u32 = 0x151419;
const FOREGROUND: u32 = 0xf7f4ee;
const MUTED: u32 = 0xaaa7a2;

#[derive(Clone, Copy)]
struct Drag {
    origin_x: f32,
    origin_y: f32,
    camera: Camera,
}

pub struct Gallery {
    folder: Option<PathBuf>,
    photos: Vec<Photo>,
    camera: Camera,
    focused: Option<usize>,
    recursive: bool,
    show_titles: bool,
    loading: bool,
    error: Option<String>,
    generation: u64,
    scan_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    drag: Option<Drag>,
    suppress_click: bool,
}

impl Gallery {
    pub fn new(initial_folder: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut gallery = Self {
            folder: None,
            photos: Vec::new(),
            camera: Camera::default(),
            focused: None,
            recursive: false,
            show_titles: true,
            loading: false,
            error: None,
            generation: 0,
            scan_task: None,
            focus_handle: cx.focus_handle().tab_stop(true),
            drag: None,
            suppress_click: false,
        };
        if let Some(folder) = initial_folder {
            gallery.load(folder, cx);
        }
        gallery
    }

    fn load(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.folder = Some(folder.clone());
        self.photos.clear();
        self.focused = None;
        self.drag = None;
        self.camera = Camera::default();
        self.loading = true;
        self.error = None;
        cx.notify();

        let recursive = self.recursive;
        let worker = cx.background_spawn(async move {
            catalog::scan_folder(&folder, recursive)
        });
        // Replacing this handle cancels stale foreground result delivery.
        self.scan_task = Some(cx.spawn(async move |weak, cx| {
            let result = worker.await;
            let _ = weak.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(photos) => view.photos = photos,
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    fn open_dialog(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // Native OS dialog. Subsequent folder enumeration runs off the UI thread.
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("选择图片文件夹")
            .pick_folder()
        {
            self.load(folder, cx);
        }
    }

    fn toggle_recursive(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.recursive = !self.recursive;
        if let Some(folder) = self.folder.clone() {
            self.load(folder, cx);
        } else {
            cx.notify();
        }
    }

    fn toggle_titles(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_titles = !self.show_titles;
        cx.notify();
    }

    fn reset_camera(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.camera = Camera::default();
        self.focused = None;
        cx.notify();
    }

    fn refresh(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(folder) = self.folder.clone() {
            self.load(folder, cx);
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.suppress_click {
            self.suppress_click = false;
            return;
        }
        if index < self.photos.len() {
            self.focused = Some(index);
            cx.notify();
        }
    }

    fn close_focus(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.focused = None;
        cx.notify();
    }

    fn next_focus(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.shift_focus(1, cx);
    }

    fn previous_focus(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.shift_focus(-1, cx);
    }

    fn shift_focus(&mut self, direction: isize, cx: &mut Context<Self>) {
        if let Some(current) = self.focused {
            let last = self.photos.len().saturating_sub(1) as isize;
            self.focused = Some((current as isize + direction).clamp(0, last) as usize);
            cx.notify();
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        if self.focused.is_some() {
            return;
        }
        self.suppress_click = false;
        self.drag = Some(Drag {
            origin_x: f32::from(event.position.x),
            origin_y: f32::from(event.position.y),
            camera: self.camera,
        });
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let dx = f32::from(event.position.x) - drag.origin_x;
        let dy = f32::from(event.position.y) - drag.origin_y;
        if dx.hypot(dy) > 6.0 {
            self.suppress_click = true;
        }
        if self.suppress_click {
            self.camera = drag.camera.translated(dx, dy);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.drag = None;
    }

    fn on_wheel(&mut self, event: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.focused.is_some() || self.photos.is_empty() {
            return;
        }
        let delta = event.delta.pixel_delta(px(28.0));
        let delta_y = f32::from(delta.y);
        let delta_x = f32::from(delta.x);
        if event.modifiers.shift {
            self.camera = self.camera.translated(delta_x + delta_y, 0.0);
        } else {
            let factor = (-delta_y * 0.0015).exp();
            let x = f32::from(event.position.x);
            let y = f32::from(event.position.y) - HEADER_HEIGHT;
            self.camera = self.camera.zoom_at(x, y, factor);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.to_lowercase();
        match key.as_str() {
            "escape" | "esc" => {
                self.focused = None;
            }
            "left" | "arrowleft" if self.focused.is_some() => {
                self.shift_focus(-1, cx);
            }
            "right" | "arrowright" if self.focused.is_some() => {
                self.shift_focus(1, cx);
            }
            "0" if self.focused.is_none() => {
                self.camera = Camera::default();
            }
            "=" | "+" | "plus" if self.focused.is_none() => {
                let (w, h) = Self::canvas_size(window);
                self.camera = self.camera.zoom_at(w * 0.5, h * 0.5, 1.2);
            }
            "-" | "minus" if self.focused.is_none() => {
                let (w, h) = Self::canvas_size(window);
                self.camera = self.camera.zoom_at(w * 0.5, h * 0.5, 1.0 / 1.2);
            }
            "up" | "arrowup" if self.focused.is_none() => {
                self.camera = self.camera.translated(0.0, 90.0);
            }
            "down" | "arrowdown" if self.focused.is_none() => {
                self.camera = self.camera.translated(0.0, -90.0);
            }
            "left" | "arrowleft" if self.focused.is_none() => {
                self.camera = self.camera.translated(90.0, 0.0);
            }
            "right" | "arrowright" if self.focused.is_none() => {
                self.camera = self.camera.translated(-90.0, 0.0);
            }
            _ => return,
        }
        cx.notify();
    }

    fn canvas_size(window: &Window) -> (f32, f32) {
        let viewport = window.viewport_size();
        (
            f32::from(viewport.width).max(1.0),
            (f32::from(viewport.height) - HEADER_HEIGHT - FOOTER_HEIGHT).max(1.0),
        )
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let location = self.folder.as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "尚未选择文件夹".to_string());
        div()
            .flex()
            .flex_row()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .px_5()
            .gap_4()
            .bg(rgb(0x101012))
            .border_b_1()
            .border_color(rgb(0x29272c))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(FOREGROUND))
                            .child("INFINITE / MOSAIC"),
                    )
                    .child(
                        div().text_size(px(11.0)).text_color(rgb(MUTED)).child(location),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("open-folder")
                            .primary()
                            .label("打开文件夹")
                            .on_click(cx.listener(Self::open_dialog)),
                    )
                    .child(
                        Button::new("refresh")
                            .label("刷新")
                            .on_click(cx.listener(Self::refresh)),
                    )
                    .child(
                        Button::new("recursive")
                            .label(if self.recursive { "递归：开" } else { "递归：关" })
                            .on_click(cx.listener(Self::toggle_recursive)),
                    )
                    .child(
                        Button::new("titles")
                            .label(if self.show_titles { "标题：开" } else { "标题：关" })
                            .on_click(cx.listener(Self::toggle_titles)),
                    )
                    .child(
                        Button::new("reset-view")
                            .label("重置视角")
                            .on_click(cx.listener(Self::reset_camera)),
                    ),
            )
            .into_any_element()
    }

    fn footer(&self) -> AnyElement {
        let status = if self.loading {
            "正在扫描文件夹…".to_string()
        } else if let Some(error) = &self.error {
            error.clone()
        } else {
            format!("{} 张图片  ·  缩放 {}%", self.photos.len(), (self.camera.zoom * 100.0).round())
        };
        div()
            .h(px(FOOTER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px_5()
            .gap_4()
            .bg(rgb(0x101012))
            .border_t_1()
            .border_color(rgb(0x29272c))
            .text_size(px(11.0))
            .text_color(rgb(MUTED))
            .child(status)
            .child("拖动探索  ·  滚轮缩放  ·  点击聚焦  ·  Esc 返回  ·  + / - / 0")
            .into_any_element()
    }

    fn tile(&self, index: usize, world_rect: Rect, cx: &mut Context<Self>) -> AnyElement {
        let photo = &self.photos[index];
        let (screen_x, screen_y) = self.camera.world_to_screen(world_rect.x, world_rect.y);
        let width = world_rect.width * self.camera.zoom;
        let height = world_rect.height * self.camera.zoom;
        let mut tile = div()
            .id(("mosaic-tile", index))
            .absolute()
            .left(px(screen_x))
            .top(px(screen_y))
            .w(px(width.max(1.0)))
            .h(px(height.max(1.0)))
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .cursor_pointer()
            .child(
                img(photo.path.as_path())
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Cover),
            );
        if self.show_titles && width >= 90.0 && height >= 75.0 {
            tile = tile.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(height.min(120.0)))
                    .bg(linear_gradient(180.0, hsla(0.0, 0.0, 0.0, 0.0), hsla(0.0, 0.0, 0.0, 0.91)))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .gap_1()
                    .p_3()
                    .child(
                        div()
                            .text_color(rgb(FOREGROUND))
                            .text_size(px(if width > 250.0 { 19.0 } else { 13.0 }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(photo.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(MUTED))
                            .child(format!("{:04}", index + 1)),
                    ),
            );
        }
        if self.focused.is_some() {
            tile = tile.child(
                div().absolute().top_0().left_0().size_full().bg(rgba(0x000000aa)),
            );
        }
        tile.on_click(cx.listener(move |view, _, _, cx| view.select(index, cx)))
            .into_any_element()
    }

    fn focus_overlay(&self, index: usize, width: f32, height: f32, cx: &mut Context<Self>) -> AnyElement {
        let photo = &self.photos[index];
        let world_rect = layout::tile_rect(index);
        let (screen_x, screen_y) = self.camera.world_to_screen(
            world_rect.x + world_rect.width * 0.5,
            world_rect.y + world_rect.height * 0.5,
        );
        let panel_w = (width - 48.0).clamp(240.0, 860.0);
        let panel_h = (height - 48.0).clamp(240.0, 680.0);
        let left = (screen_x - panel_w / 2.0).clamp(24.0, (width - panel_w - 24.0).max(24.0));
        let top = (screen_y - panel_h / 2.0).clamp(24.0, (height - panel_h - 24.0).max(24.0));

        div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(width))
            .h(px(height))
            .child(
                div()
                    .id("focus-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(rgba(0x000000b0))
                    .on_click(cx.listener(Self::close_focus)),
            )
            .child(
                div()
                    .id("focus-panel")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(panel_w))
                    .h(px(panel_h))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(0xccc8c2))
                    .bg(rgb(0x111114))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(46.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .bg(rgb(0x19181c))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(FOREGROUND))
                                    .child(format!("{} / {}", index + 1, self.photos.len())),
                            )
                            .child(
                                Button::new("focus-close")
                                    .label("关闭 ×")
                                    .on_click(cx.listener(Self::close_focus)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x080809))
                            .child(
                                img(photo.path.as_path())
                                    .w_full()
                                    .h_full()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    )
                    .child(
                        div()
                            .h(px(64.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .px_4()
                            .bg(rgb(0x19181c))
                            .child(
                                Button::new("focus-prev")
                                    .label("← 上一张")
                                    .on_click(cx.listener(Self::previous_focus)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_size(px(13.0))
                                    .text_color(rgb(FOREGROUND))
                                    .child(photo.title.clone()),
                            )
                            .child(
                                Button::new("focus-next")
                                    .label("下一张 →")
                                    .on_click(cx.listener(Self::next_focus)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn canvas(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (width, height) = Self::canvas_size(window);
        let mut surface = div()
            .id("mosaic-wall")
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_wheel));

        if self.photos.is_empty() {
            let message = if self.loading {
                "正在读取图片…"
            } else if self.error.is_some() {
                "当前文件夹无法访问，请重试或选择其他文件夹"
            } else if self.folder.is_some() {
                "当前文件夹没有受支持的图片"
            } else {
                "选择一个文件夹，开始探索你的影像世界"
            };
            surface = surface.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(34.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(FOREGROUND))
                            .child("IMAGES, UNBOUND."),
                    )
                    .child(
                        div().text_size(px(14.0)).text_color(rgb(MUTED)).child(message),
                    )
                    .child(
                        Button::new("empty-open")
                            .primary()
                            .label("打开图片文件夹")
                            .on_click(cx.listener(Self::open_dialog)),
                    ),
            );
        } else {
            for tile in layout::visible_tiles(self.photos.len(), self.camera, width, height) {
                surface = surface.child(self.tile(tile.index, tile.rect, cx));
            }
            if let Some(index) = self.focused {
                surface = surface.child(self.focus_overlay(index, width, height, cx));
            }
        }
        surface.into_any_element()
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(self.header(cx))
            .child(self.canvas(window, cx))
            .child(self.footer())
    }
}
