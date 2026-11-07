//! Native, read-only filesystem notifications with a bounded event mailbox.
//! File change events only request a rescan; all catalog and UI mutations stay
//! on the GPUI executor.
use std::path::Path;
use async_channel::Sender;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};

use crate::catalog::is_photo;

/// Directory creates, deletes, or moves can introduce/remove many images.
/// Ordinary writes only matter for supported image extensions.
pub fn affects_gallery(event: &Event) -> bool {
    match &event.kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Name(_)) => true,
        EventKind::Modify(_) => event.paths.iter().any(|path| is_photo(path)),
        _ => false,
    }
}

pub fn watch_folder(
    folder: &Path,
    recursive: bool,
    notification: Sender<()>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if let Ok(event) = result {
            if affects_gallery(&event) {
                // Bounded to one pending notification, so large copy/move operations
                // cannot grow memory while scanning or rendering is busy.
                let _ = notification.try_send(());
            }
        }
    })?;
    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher.watch(folder, mode)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, RemoveKind, DataChange, RenameMode};

    fn event(kind: EventKind, path: &str) -> Event {
        Event::new(kind).add_path(path.into())
    }

    #[test]
    fn image_edits_trigger_rescan_but_unrelated_writes_do_not() {
        let edited = event(EventKind::Modify(ModifyKind::Data(DataChange::Any)), "photo.JPEG");
        let notes = event(EventKind::Modify(ModifyKind::Data(DataChange::Any)), "readme.md");
        assert!(affects_gallery(&edited));
        assert!(!affects_gallery(&notes));
    }

    #[test]
    fn folder_creation_removal_and_renaming_are_relevant() {
        assert!(affects_gallery(&event(EventKind::Create(CreateKind::Folder), "new.folder")));
        assert!(affects_gallery(&event(EventKind::Remove(RemoveKind::Folder), "old.folder")));
        assert!(affects_gallery(&event(EventKind::Modify(ModifyKind::Name(RenameMode::Both)), "photo.jpg")));
    }

    #[test]
    fn access_events_do_not_trigger_rescans() {
        assert!(!affects_gallery(&event(EventKind::Access(notify::event::AccessKind::Any), "photo.png")));
    }
}
