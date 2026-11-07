//! Read-only image discovery. No sidecar files are written to the source folder.
use std::{fs, path::{Path, PathBuf}};

#[derive(Clone, Debug)]
pub struct Photo {
    pub path: PathBuf,
    pub title: String,
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
                photos.push(Photo { path, title });
            }
        }
    }
    photos.sort_by(|a, b| a.path.to_string_lossy()
        .to_lowercase().cmp(&b.path.to_string_lossy().to_lowercase())
        .then_with(|| a.path.cmp(&b.path)));
    Ok(photos)
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
