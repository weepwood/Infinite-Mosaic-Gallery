//! GPUI native presentation. Geometry, image catalog and camera math are testable
//! without instantiating a GPU window.
use std::{collections::{HashMap, HashSet}, path::PathBuf, time::{Duration, Instant}};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use crate::camera::Camera;
use crate::catalog::{self, Photo};
use crate::layout::{self, Rect, Tile};
use crate::{thumbnails, image_cache::RecentImageCache, reflow, focus_geometry, watcher, slots, scan_gate::ScanGate};

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
    /// Sorted persistent spatial positions for binary-search viewport culling.
    slots: Vec<usize>,
    camera: Camera,
    focused: Option<usize>,
    /// Last selected item remains mounted until the closing spring reaches zero.
    presented: Option<usize>,
    recursive: bool,
    show_titles: bool,
    loading: bool,
    error: Option<String>,
    generation: u64,
    scan_task: Option<Task<()>>,
    scan_gate: ScanGate,
    watcher: Option<notify::RecommendedWatcher>,
    watcher_task: Option<Task<()>>,
    watcher_session: u64,
    live_watch: bool,
    watcher_error: Option<String>,
    images: Entity<RecentImageCache>,
    focus_images: Entity<RecentImageCache>,
    preview_paths: HashMap<usize, PathBuf>,
    preview_failures: HashMap<usize, String>,
    preview_inflight: HashSet<usize>,
    last_preview_check: Instant,
    focus_handle: FocusHandle,
    drag: Option<Drag>,
    suppress_click: bool,
}

impl Gallery {
    pub fn new(initial_folder: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut gallery = Self {
            folder: None,
            photos: Vec::new(),
            slots: Vec::new(),
            camera: Camera::default(),
            focused: None,
            presented: None,
            recursive: false,
            show_titles: true,
            loading: false,
            error: None,
            generation: 0,
            scan_task: None,
            scan_gate: ScanGate::default(),
            watcher: None,
            watcher_task: None,
            watcher_session: 0,
            live_watch: true,
            watcher_error: None,
            images: RecentImageCache::new(128, cx),
            focus_images: RecentImageCache::new(2, cx),
            preview_paths: HashMap::new(),
            preview_failures: HashMap::new(),
            preview_inflight: HashSet::new(),
            last_preview_check: Instant::now(),
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
        self.scan(folder, false, cx);
        self.configure_watcher(cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(folder) = self.folder.clone() {
            self.scan(folder, true, cx);
        }
    }

    /// A native watcher owns one bounded notification channel and a foreground
    /// task. Switching folders or toggling recursion retires the old session.
    fn configure_watcher(&mut self, cx: &mut Context<Self>) {
        self.watcher_session = self.watcher_session.wrapping_add(1);
        self.watcher_task = None;
        self.watcher = None;
        self.watcher_error = None;
        if !self.live_watch {
            cx.notify();
            return;
        }
        let Some(folder) = self.folder.as_ref() else { return };
        let (sender, receiver) = async_channel::bounded::<()>(1);
        let watcher = match watcher::watch_folder(folder, self.recursive, sender) {
            Ok(watcher) => watcher,
            Err(err) => {
                self.watcher_error = Some(format!("自动监听不可用：{err}"));
                cx.notify();
                return;
            }
        };
        self.watcher = Some(watcher);
        let session = self.watcher_session;
        self.watcher_task = Some(cx.spawn(async move |weak, cx| {
            while receiver.recv().await.is_ok() {
                // Queue capacity 1 keeps large copy/move bursts memory bounded.
                cx.background_executor()
                    .timer(Duration::from_millis(550))
                    .await;
                while receiver.try_recv().is_ok() {}
                if weak.update(cx, |gallery, cx| {
                    if gallery.live_watch && gallery.watcher_session == session {
                        gallery.reload(cx);
                    }
                }).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn toggle_watch(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.live_watch = !self.live_watch;
        self.configure_watcher(cx);
    }

    fn scan(&mut self, folder: PathBuf, preserve_view: bool, cx: &mut Context<Self>) {
        // Coalesce watcher bursts while a scan runs, rather than launching an
        // unbounded number of background directory enumerations.
        if !self.scan_gate.request(preserve_view) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.folder = Some(folder.clone());
        if !preserve_view {
            self.photos.clear();
            self.slots.clear();
            self.preview_paths.clear();
            self.preview_failures.clear();
            self.preview_inflight.clear();
            self.focused = None;
            self.presented = None;
            self.drag = None;
            self.camera = Camera::default();
        }
        self.loading = true;
        self.error = None;
        cx.notify();

        let recursive = self.recursive;
        let worker = cx.background_spawn(async move {
            slots::scan_with_slots(&folder, recursive)
        });
        // Stale results cannot replace a later folder/refresh request.
        self.scan_task = Some(cx.spawn(async move |weak, cx| {
            let result = worker.await;
            let _ = weak.update(cx, |view, cx| {
                if view.generation != generation { return; }
                view.loading = false;
                match result {
                    Ok(photos) => {
                        // Preserve identity and thumbnails across filesystem events.
                        // A file's size/mtime stamp must match to reuse decoded pixels.
                        let old_photos = std::mem::take(&mut view.photos);
                        let mut ready_by_path = HashMap::new();
                        for (old_index, preview_path) in view.preview_paths.drain() {
                            if let Some(photo) = old_photos.get(old_index) {
                                if let Some(stamp) = photo.stamp {
                                    ready_by_path.insert(photo.path.clone(), (stamp, preview_path));
                                }
                            }
                        }
                        let selected_path = view.focused
                            .and_then(|index| old_photos.get(index))
                            .map(|photo| photo.path.clone());
                        // Photos are already sorted by persistent spatial slot.
                        // Deletions leave tombstones and never shift survivors.
                        view.photos = photos;
                        view.slots = view.photos.iter().map(|photo| photo.slot).collect();
                        for (index, photo) in view.photos.iter().enumerate() {
                            if let (Some(new_stamp), Some((old_stamp, preview_path))) =
                                (photo.stamp, ready_by_path.get(&photo.path))
                            {
                                if new_stamp == *old_stamp {
                                    view.preview_paths.insert(index, preview_path.clone());
                                }
                            }
                        }
                        view.focused = catalog::restore_selected_index(
                            selected_path.as_deref(),
                            &view.photos,
                        );
                        view.presented = view.focused;
                        // Focus uses full originals keyed by path. The same path can
                        // contain new bytes after an edit, so reset just that cache.
                        view.focus_images = RecentImageCache::new(2, cx);
                        // Thumbnail resources have versioned disk paths: unchanged
                        // previews can safely stay in the bounded GPU cache.
                        view.preview_failures.clear();
                        view.preview_inflight.clear();
                        view.last_preview_check = Instant::now();
                    }
                    Err(error) => {
                        // For refresh failures, leave previously visible photos
                        // and the camera untouched instead of blanking the wall.
                        view.error = Some(error);
                    }
                }
                if view.scan_gate.complete() {
                    // Exactly one follow-up scan for all notifications received
                    // during this scan; a newer folder selection supersedes it.
                    view.reload(cx);
                }
                cx.notify();
            });
        }));
    }

    fn open_dialog(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        // GPUI's asynchronous platform picker keeps the native window responsive.
        let chooser = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择图片文件夹".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let folder = chooser.await.ok()?.ok()??.into_iter().next()?;
            window.update(|_, cx| {
                view.update(cx, |gallery, cx| gallery.load(folder, cx));
            }).ok()
        }).detach();
    }

    fn toggle_recursive(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.recursive = !self.recursive;
        if self.folder.is_some() {
            self.configure_watcher(cx);
            self.reload(cx);
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
        self.reload(cx);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.suppress_click {
            self.suppress_click = false;
            return;
        }
        if index < self.photos.len() {
            self.focused = Some(index);
            self.presented = Some(index);
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
            let index = (current as isize + direction).clamp(0, last) as usize;
            self.focused = Some(index);
            self.presented = Some(index);
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
                        Button::new("watcher-toggle")
                            .label(if self.live_watch { "自动同步：开" } else { "自动同步：关" })
                            .on_click(cx.listener(Self::toggle_watch)),
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
        } else if let Some(error) = &self.watcher_error {
            format!("{error}（可继续手动刷新）")
        } else {
            let reserved = self.slots.last().map(|slot| slot + 1 - self.slots.len()).unwrap_or(0);
            format!("{} 张图片  ·  预留空槽 {}  ·  缩放 {}%  ·  预览已就绪 {}  ·  处理中 {}  ·  失败 {}",
                self.photos.len(), reserved, (self.camera.zoom * 100.0).round(),
                self.preview_paths.len(), self.preview_inflight.len(), self.preview_failures.len())
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


    /// Only decode near-viewport pictures, with a hard bound on outstanding workers.
    /// Render may be called multiple times; inflight/ready/failed state deduplicates it.
    fn request_previews(&mut self, tiles: &[Tile], width: f32, height: f32, cx: &mut Context<Self>) {
        const MAX_INFLIGHT: usize = 3;
        // Disk LRU pruning may remove older PNGs still indexed in memory.
        // Check locally cached paths occasionally, not on every render frame.
        if self.last_preview_check.elapsed() >= Duration::from_secs(3) {
            for tile in tiles {
                if self.preview_paths.get(&tile.index)
                    .is_some_and(|path| !path.is_file())
                {
                    self.preview_paths.remove(&tile.index);
                }
            }
            self.last_preview_check = Instant::now();
        }
        let center_x = width * 0.5;
        let center_y = height * 0.5;
        let mut sorted = tiles.to_vec();
        sorted.sort_by(|left, right| {
            let distance = |tile: &Tile| {
                let (x, y) = self.camera.world_to_screen(
                    tile.rect.x + tile.rect.width * 0.5,
                    tile.rect.y + tile.rect.height * 0.5,
                );
                (x - center_x).powi(2) + (y - center_y).powi(2)
            };
            distance(left).total_cmp(&distance(right))
        });

        for tile in sorted {
            if self.preview_inflight.len() >= MAX_INFLIGHT {
                break;
            }
            let index = tile.index;
            if self.preview_paths.contains_key(&index)
                || self.preview_failures.contains_key(&index)
                || !self.preview_inflight.insert(index)
            {
                continue;
            }

            let path = self.photos[index].path.clone();
            let generation = self.generation;
            let worker = cx.background_spawn(async move {
                thumbnails::generate_preview(&path)
            });
            cx.spawn(async move |weak, cx| {
                let result = worker.await;
                let _ = weak.update(cx, |gallery, cx| {
                    if gallery.generation != generation {
                        return;
                    }
                    gallery.preview_inflight.remove(&index);
                    match result {
                        Ok(path) => { gallery.preview_paths.insert(index, path); }
                        Err(error) => { gallery.preview_failures.insert(index, error); }
                    }
                    cx.notify();
                });
            }).detach();
        }
    }

    fn tile(&self, index: usize, target: Rect, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let photo = &self.photos[index];
        // Each tile owns four stable, interruptible spring channels.
        // GPUI Base motion honors the platform's reduced-motion preference.
        let spring = gpui_kit::base::Spring::new(Duration::from_millis(390)).with_damping(0.84);
        let x = gpui_kit::base::spring((photo.slot, "wall-x"), target.x, spring, window, cx);
        let y = gpui_kit::base::spring((photo.slot, "wall-y"), target.y, spring, window, cx);
        let w = gpui_kit::base::spring((photo.slot, "wall-width"), target.width, spring, window, cx);
        let h = gpui_kit::base::spring((photo.slot, "wall-height"), target.height, spring, window, cx);
        let (screen_x, screen_y) = self.camera.world_to_screen(x, y);
        let width = w * self.camera.zoom;
        let height = h * self.camera.zoom;
        let art: AnyElement = if let Some(preview) = self.preview_paths.get(&index) {
            img(preview.as_path())
                .id(("thumbnail-image", photo.slot))
                .w_full()
                .h_full()
                .object_fit(ObjectFit::Cover)
                .with_loading(|| {
                    div().size_full().bg(rgb(SURFACE)).into_any_element()
                })
                .with_fallback(|| {
                    div().size_full().bg(rgb(SURFACE))
                        .flex().items_center().justify_center()
                        .text_size(px(11.0)).text_color(rgb(MUTED))
                        .child("缩略图读取失败").into_any_element()
                })
                .into_any_element()
        } else {
            div().size_full().bg(rgb(SURFACE))
                .flex().items_center().justify_center()
                .text_size(px(11.0)).text_color(rgb(MUTED))
                .child(if self.preview_failures.contains_key(&index) {
                    "图片不可预览"
                } else {
                    "LOADING"
                })
                .into_any_element()
        };
        let mut tile = div()
            .id(("mosaic-tile", photo.slot))
            .absolute()
            .left(px(screen_x))
            .top(px(screen_y))
            .w(px(width.max(1.0)))
            .h(px(height.max(1.0)))
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .cursor_pointer()
            .child(art);
        if self.show_titles && width >= 90.0 && height >= 75.0 {
            tile = tile.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(height.min(120.0)))
                    .bg(linear_gradient(180.0, linear_color_stop(hsla(0.0, 0.0, 0.0, 0.0), 0.0), linear_color_stop(hsla(0.0, 0.0, 0.0, 0.91), 1.0)))
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
        // Dimming is owned by the shared-space backdrop; the underlying
        // tile never changes its color abruptly during the transition.
        tile.on_click(cx.listener(move |view, _, _, cx| view.select(index, cx)))
            .into_any_element()
    }

    /// Shared-space inspector. It begins at the exact screen-space tile bounds,
    /// morphs to a centered lightbox, and reverses to the same tile on close.
    /// Keeping the last selected index mounted preserves the exit animation.
    fn focus_overlay(
        &self,
        index: usize,
        width: f32,
        height: f32,
        progress: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let photo = &self.photos[index];
        let rect = focus_geometry::focus_rect(
            layout::tile_rect(photo.slot), self.camera, width, height, progress,
        );
        let show_controls = self.focused.is_some() && progress >= 0.88;
        let showing_full_image = self.focused.is_some() && progress >= 0.94;
        let preview = self.preview_paths.get(&index).cloned();
        let source = if showing_full_image {
            photo.path.clone()
        } else {
            preview.clone().unwrap_or_else(|| photo.path.clone())
        };

        let image = img(source.as_path())
            .id(("focus-hero", photo.slot))
            .w_full()
            .h_full()
            .object_fit(if progress >= 0.88 { ObjectFit::Contain } else { ObjectFit::Cover })
            .with_loading(move || {
                if let Some(path) = preview.as_ref() {
                    img(path.as_path())
                        .w_full().h_full().object_fit(ObjectFit::Contain)
                        .into_any_element()
                } else {
                    div().size_full()
                        .flex().items_center().justify_center()
                        .text_size(px(12.0)).text_color(rgb(MUTED))
                        .child("正在加载图片…").into_any_element()
                }
            })
            .with_fallback(|| {
                div().size_full()
                    .flex().items_center().justify_center()
                    .text_color(rgb(MUTED))
                    .child("无法读取图片").into_any_element()
            });

        let mut panel = div()
            .id("focus-panel")
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.width.max(1.0)))
            .h(px(rect.height.max(1.0)))
            .overflow_hidden()
            .bg(rgb(0x0d0d0f))
            .border_1()
            .border_color(rgb(0x252329))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(image);

        if show_controls {
            panel = panel
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(48.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_4()
                        .bg(rgba(0x111114df))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(FOREGROUND))
                                .child(format!("{:04} / {:04}", index + 1, self.photos.len())),
                        )
                        .child(
                            Button::new("focus-close")
                                .label("关闭 ×")
                                .on_click(cx.listener(Self::close_focus)),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(62.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_4()
                        .bg(rgba(0x111114eb))
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
                );
        }
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
                    .bg(rgba(0x000000c8))
                    .opacity(progress.clamp(0.0, 1.0))
                    .on_click(cx.listener(Self::close_focus)),
            )
            .child(panel)
            .into_any_element()
    }

    fn canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (width, height) = Self::canvas_size(window);
        // Sample even while closed, so the first click animates from zero.
        // GPUI Base handles retargeting and the OS reduced-motion setting.
        let progress = gpui_kit::base::spring(
            "gallery-focus-morph",
            if self.focused.is_some() { 1.0f32 } else { 0.0f32 },
            gpui_kit::base::Spring::new(Duration::from_millis(430)).with_damping(0.88),
            window,
            cx,
        ).clamp(0.0, 1.0);
        if self.focused.is_none() && progress <= 0.001 {
            self.presented = None;
        }
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
            let tiles = layout::visible_tiles(&self.slots, self.camera, width, height);
            self.request_previews(&tiles, width, height, cx);
            let targets: HashMap<usize, Rect> = self.focused
                .map(|focus| reflow::focus_reflow(&self.slots, focus)
                    .into_iter().map(|tile| (tile.index, tile.rect)).collect())
                .unwrap_or_default();
            for tile in tiles {
                let target = targets.get(&tile.index).copied().unwrap_or(tile.rect);
                surface = surface.child(self.tile(tile.index, target, window, cx));
            }
            if let Some(index) = self.presented.filter(|_| progress > 0.001) {
                // Keep the outgoing photo until the morph fully returns to its tile.
                surface = surface.child(
                    image_cache(self.focus_images.clone())
                        .child(self.focus_overlay(index, width, height, progress, cx)),
                );
            }
        }
        // Never use the App-wide retain-all cache for an unbounded photo collection.
        image_cache(self.images.clone()).child(surface).into_any_element()
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
