//! Read-only image discovery. No sidecar files are written to the source folder.
use std::{collections::HashMap, fs, path::{Path, PathBuf}, time::SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Clone, Debug)]
pub struct Photo {
    pub path: PathBuf,
    pub title: String,
    /// Optional metadata collected off-thread, for safe preview reuse after rescans.
    pub stamp: Option<FileStamp>,
}

pub fn is_photo(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" | "tif" | "tiff"
    )
}

/// Enumerates files off the UI executor. Symlinks are not followed.
pub fn scan_folder(folder: &Path, recursive: bool) -> Result<Vec<Photo>, String> {
    if !folder.is_dir() {
        return Err(format!("文件夹不存在或无法访问：{}", folder.display()));
    }

    let mut pending = vec![folder.to_path_buf()];
    let mut photos = Vec::new();
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(err) if dir == folder => return Err(format!("无法读取文件夹：{err}")),
            Err(_) => continue, // An inaccessible subdirectory must not hide other photos.
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() && recursive {
                pending.push(path);
            } else if kind.is_file() && is_photo(&path) {
                let title = path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "未命名图片".to_string());
                let stamp = entry.metadata().ok().and_then(|metadata| {
                    metadata.modified().ok().map(|modified| FileStamp {
                        size: metadata.len(),
                        modified,
                    })
                });
                photos.push(Photo { path, title, stamp });
            }
        }
    }
    photos.sort_by(|a, b| a.path.to_string_lossy()
        .to_lowercase().cmp(&b.path.to_string_lossy().to_lowercase())
        .then_with(|| a.path.cmp(&b.path)));
    Ok(photos)
}

/// Keeps existing tiles in the same spatial slots when new photos appear.
pub fn preserve_order(previous: &[Photo], discovered: Vec<Photo>) -> Vec<Photo> {
    let mut by_path: HashMap<PathBuf, Photo> = discovered
        .into_iter().map(|photo| (photo.path.clone(), photo)).collect();
    let mut ordered = Vec::with_capacity(by_path.len());
    for old in previous {
        if let Some(updated) = by_path.remove(&old.path) {
            ordered.push(updated);
        }
    }
    // New photos enter at the end, in deterministic filename order.
    let mut appended: Vec<Photo> = by_path.into_values().collect();
    appended.sort_by(|a, b| {
        a.path.to_string_lossy().to_lowercase()
            .cmp(&b.path.to_string_lossy().to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    ordered.extend(appended);
    ordered
}

/// Finds an existing selection by path rather than its former sort position.
pub fn restore_selected_index(selected: Option<&Path>, photos: &[Photo]) -> Option<usize> {
    selected.and_then(|path| photos.iter().position(|photo| photo.path == path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(is_photo(Path::new("IMG_1.JpG")));
        assert!(is_photo(Path::new("中文图片.webp")));
        assert!(!is_photo(Path::new("notes.txt")));
        assert!(!is_photo(Path::new("foo")));
    }

    #[test]
    fn selection_survives_insertions_and_deletions() {
        let make = |name: &str| Photo {
            path: PathBuf::from(name),
            title: name.to_string(),
            stamp: None,
        };
        let original = vec![make("b.jpg"), make("z.jpg")];
        let updated = vec![make("a.jpg"), make("b.jpg"), make("z.jpg")];
        let selected = original[1].path.as_path();
        assert_eq!(restore_selected_index(Some(selected), &updated), Some(2));
        assert_eq!(restore_selected_index(Some(selected), &updated[..2]), None);
        assert_eq!(restore_selected_index(None, &updated), None);
    }

    #[test]
    fn file_watcher_rescan_preserves_existing_spatial_order() {
        let photo = |name: &str| Photo {
            path: PathBuf::from(name),
            title: name.to_string(),
            stamp: None,
        };
        let old = vec![photo("c.jpg"), photo("z.jpg"), photo("m.jpg")];
        let incoming = vec![
            photo("a.jpg"), photo("b.jpg"), photo("c.jpg"),
            photo("m.jpg"), photo("z.jpg"),
        ];
        let merged = preserve_order(&old, incoming);
        let paths: Vec<_> = merged.iter().map(|p| p.path.as_os_str().to_str().unwrap()).collect();
        assert_eq!(paths, ["c.jpg", "z.jpg", "m.jpg", "a.jpg", "b.jpg"]);
    }

    #[test]
    fn removed_files_disappear_and_remaining_photos_stay_ordered() {
        let photo = |name: &str| Photo {
            path: PathBuf::from(name),
            title: name.to_string(),
            stamp: None,
        };
        let merged = preserve_order(&[photo("a.png"), photo("b.png")], vec![photo("b.png")]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].path, PathBuf::from("b.png"));
    }

    #[test]
    fn nested_directories_are_opt_in() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("mosaic-test-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("a.JPG"), b"test").unwrap();
        fs::write(root.join("nested").join("b.png"), b"test").unwrap();
        assert_eq!(scan_folder(&root, false).unwrap().len(), 1);
        assert_eq!(scan_folder(&root, true).unwrap().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
