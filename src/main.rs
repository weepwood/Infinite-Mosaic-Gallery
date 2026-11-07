mod app;
mod camera;
mod catalog;
mod image_cache;
mod reflow;
mod focus_geometry;
mod watcher;
mod slots;
mod scan_gate;
mod thumbnails;
mod layout;

use std::path::PathBuf;
use gpui_kit::*;

fn main() {
    let initial_folder = std::env::args_os().nth(1).map(PathBuf::from);
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |window, cx| {
            window.set_window_title("Infinite Mosaic Gallery");
            cx.new(|cx| app::Gallery::new(initial_folder.clone(), cx))
        })
        .expect("无法创建 Infinite Mosaic Gallery 窗口");
    });
}
