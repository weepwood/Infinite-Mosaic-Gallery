//! Per-library persistent spatial identities.
//! Records only relative paths and immutable numerical slots under the *app*
//! data directory. User photo folders remain strictly read-only.
//!
//! Removing a photo leaves its slot reserved: other photos never jump.
//! If the same relative path reappears, it reclaims its original slot.
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufReader, BufWriter},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use serde::{Deserialize, Serialize};
use crate::catalog::{self, Photo};

const SCHEMA_VERSION: u32 = 1;
const MAX_SLOTS: usize = 1_000_000;
const MAX_MANIFEST_BYTES: u64 = 128 * 1024 * 1024;
static MANIFEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    root: PathBuf,
    /// Index in this array is a stable slot; missing files keep their path as
    /// a tombstone, so they are not replaced by newly discovered photographs.
    paths: Vec<PathBuf>,
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("InfiniteMosaicGallery")
        .join("slots-v1")
}

fn manifest_path(root: &Path, data_dir: &Path) -> PathBuf {
    let digest = blake3::hash(root.as_os_str().as_encoded_bytes());
    data_dir.join(format!("{}.json", digest.to_hex()))
}

fn valid_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|part| matches!(part, Component::Normal(_)))
}

fn read_manifest(path: &Path, root: &Path) -> Result<Manifest, String> {
    if !path.exists() {
        return Ok(Manifest { version: SCHEMA_VERSION, root: root.to_path_buf(), paths: Vec::new() });
    }
    let metadata = fs::metadata(path).map_err(|err| format!("读取图片空间索引失败：{err}"))?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("图片空间索引超出允许大小，请备份并检查索引文件".into());
    }
    let file = fs::File::open(path).map_err(|err| format!("打开图片空间索引失败：{err}"))?;
    let manifest: Manifest = serde_json::from_reader(BufReader::new(file))
        .map_err(|err| format!("空间索引损坏：{err}。请手动备份后处理，不会覆盖原索引"))?;
    if manifest.version != SCHEMA_VERSION || manifest.root != root || manifest.paths.len() > MAX_SLOTS {
        return Err("空间索引版本或图库路径不匹配；不会覆盖现有索引".into());
    }
    let mut seen = HashSet::new();
    for path in &manifest.paths {
        if !valid_relative(path) || !seen.insert(path) {
            return Err("空间索引包含重复或非法的相对路径；不会覆盖现有索引".into());
        }
    }
    Ok(manifest)
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let dir = path.parent().ok_or("空间索引目录无效")?;
    fs::create_dir_all(dir).map_err(|err| format!("创建应用数据目录失败：{err}"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)
        .map_err(|err| format!("创建临时空间索引失败：{err}"))?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        serde_json::to_writer(&mut writer, manifest)
            .map_err(|err| format!("写入空间索引失败：{err}"))?;
    }
    temporary.as_file_mut().sync_all()
        .map_err(|err| format!("同步空间索引失败：{err}"))?;
    // Atomic rename, including when replacing the previous snapshot.
    temporary.persist(path)
        .map_err(|err| format!("保存空间索引失败：{}", err.error))?;
    Ok(())
}

/// Caller must hold the process-wide manifest lock.
fn allocate(
    root: &Path,
    data_dir: &Path,
    mut discovered: Vec<Photo>,
) -> Result<Vec<Photo>, String> {
    let path = manifest_path(root, data_dir);
    let mut manifest = read_manifest(&path, root)?;
    let mut slot_by_path: HashMap<PathBuf, usize> = manifest.paths.iter()
        .enumerate().map(|(slot, path)| (path.clone(), slot)).collect();
    let mut changed = false;

    for photo in &mut discovered {
        let relative = photo.path.strip_prefix(root)
            .map_err(|_| "图片不在所选文件夹内".to_string())?;
        if !valid_relative(relative) {
            return Err("图片路径不能用作有效的相对索引".into());
        }
        let slot = if let Some(&slot) = slot_by_path.get(relative) {
            slot
        } else {
            if manifest.paths.len() >= MAX_SLOTS {
                return Err("空间索引槽位已达到上限，请先整理图库".into());
            }
            let slot = manifest.paths.len();
            let name = relative.to_path_buf();
            manifest.paths.push(name.clone());
            slot_by_path.insert(name, slot);
            changed = true;
            slot
        };
        photo.slot = slot;
    }

    if changed {
        write_manifest(&path, &manifest)?;
    }
    discovered.sort_by_key(|photo| photo.slot);
    Ok(discovered)
}

/// Scan and index outside the UI executor. Concurrent rescan tasks serialize
/// their manifest read-modify-write operation, including after watcher bursts.
pub fn scan_with_slots(folder: &Path, recursive: bool) -> Result<Vec<Photo>, String> {
    let root = fs::canonicalize(folder).map_err(|err| format!("无法访问图片文件夹：{err}"))?;
    let discovered = catalog::scan_folder(&root, recursive)?;
    let lock = MANIFEST_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
    allocate(&root, &data_dir(), discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(path: PathBuf) -> Photo {
        Photo { path, title: String::new(), stamp: None, slot: 0 }
    }

    #[test]
    fn adding_and_deleting_never_moves_an_existing_photo() {
        let workspace = tempfile::tempdir().unwrap();
        let data = workspace.path().join("data");
        let root = workspace.path().join("album");
        fs::create_dir_all(&root).unwrap();
        let a = root.join("a.jpg");
        let b = root.join("b.jpg");
        let c = root.join("c.jpg");
        let first = allocate(&root, &data, vec![photo(a.clone()), photo(b.clone())]).unwrap();
        assert_eq!(first.iter().map(|p| p.slot).collect::<Vec<_>>(), vec![0, 1]);
        let second = allocate(&root, &data, vec![photo(b.clone()), photo(c.clone())]).unwrap();
        assert_eq!(second.iter().map(|p| (p.path.clone(), p.slot)).collect::<Vec<_>>(),
                   vec![(b.clone(), 1), (c.clone(), 2)]);
        let third = allocate(&root, &data, vec![photo(a.clone()), photo(c.clone())]).unwrap();
        assert_eq!(third.iter().map(|p| p.slot).collect::<Vec<_>>(), vec![0, 2]);
        let restored = allocate(&root, &data, vec![photo(b), photo(c)]).unwrap();
        assert_eq!(restored.iter().map(|p| p.slot).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn two_folders_have_independent_slot_manifests() {
        let workspace = tempfile::tempdir().unwrap();
        let data = workspace.path().join("data");
        let a = workspace.path().join("A");
        let b = workspace.path().join("B");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        allocate(&a, &data, vec![photo(a.join("z.png"))]).unwrap();
        let b_images = allocate(&b, &data, vec![photo(b.join("a.png"))]).unwrap();
        assert_eq!(b_images[0].slot, 0);
        assert_ne!(manifest_path(&a, &data), manifest_path(&b, &data));
    }

    #[test]
    fn malformed_manifest_is_not_overwritten() {
        let workspace = tempfile::tempdir().unwrap();
        let data = workspace.path().join("data");
        let root = workspace.path().join("images");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&data).unwrap();
        let file = manifest_path(&root, &data);
        fs::write(&file, b"not JSON").unwrap();
        assert!(allocate(&root, &data, vec![photo(root.join("ok.jpg"))]).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"not JSON");
    }

    #[test]
    fn rejects_paths_that_escape_root_or_duplicate_entries() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("album");
        let data = workspace.path().join("data");
        fs::create_dir_all(&root).unwrap();
        assert!(allocate(&root, &data, vec![photo(workspace.path().join("outside.png"))]).is_err());
        assert!(!valid_relative(Path::new("../escape.jpg")));
        assert!(!valid_relative(Path::new("")));
    }

    #[test]
    fn real_scan_persists_across_rescans() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("album");
        let data = workspace.path().join("data");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.jpg"), b"not decoded while scanning").unwrap();
        fs::write(root.join("b.png"), b"not decoded while scanning").unwrap();
        let root = fs::canonicalize(root).unwrap();
        let first = allocate(&root, &data, catalog::scan_folder(&root, false).unwrap()).unwrap();
        assert_eq!(first.len(), 2);
        fs::remove_file(root.join("a.jpg")).unwrap();
        fs::write(root.join("c.jpg"), b"just a catalog entry").unwrap();
        let second = allocate(&root, &data, catalog::scan_folder(&root, false).unwrap()).unwrap();
        assert_eq!(second.iter().map(|p| p.slot).collect::<Vec<_>>(), vec![1, 2]);
    }
}
