//! On-demand persistent previews. Reads source folders without modifying them.
use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::UNIX_EPOCH,
};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

const PREVIEW_EDGE: u32 = 768;
const CACHE_BUDGET: u64 = 512 * 1024 * 1024;
const DECODE_BUDGET: u64 = 192 * 1024 * 1024;
static WRITES: AtomicUsize = AtomicUsize::new(0);

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir)
        .join("InfiniteMosaicGallery").join("previews-v1")
}

fn cache_key(path: &Path, metadata: &fs::Metadata) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(path.as_os_str().as_encoded_bytes());
    hasher.update(&metadata.len().to_le_bytes());
    let modified_nanos = metadata.modified().ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    hasher.update(&modified_nanos.to_le_bytes());
    hasher.update(&PREVIEW_EDGE.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Builds one alpha-preserving PNG on a background worker, publishing it atomically.
pub fn generate_preview(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::metadata(path).map_err(|err| format!("读取图片信息失败：{err}"))?;
    if !metadata.is_file() {
        return Err("图片路径不是普通文件".into());
    }
    let directory = cache_dir();
    let output = directory.join(format!("{}.png", cache_key(path, &metadata)));
    if fs::metadata(&output).is_ok_and(|m| m.len() > 0) {
        return Ok(output);
    }

    let reader = ImageReader::open(path)
        .map_err(|err| format!("图片无法打开：{err}"))?
        .with_guessed_format()
        .map_err(|err| format!("图片格式识别失败：{err}"))?;
    let mut decoder = reader.into_decoder()
        .map_err(|err| format!("图片无法解码：{err}"))?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODE_BUDGET);
    decoder.set_limits(limits).map_err(|err| format!("图片超出解码限额：{err}"))?;
    let orientation = decoder.orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|err| format!("图片解码失败：{err}"))?;
    image.apply_orientation(orientation);
    // The image crate's thumbnail() may upscale tiny images; never do that.
    let resized = if image.width() <= PREVIEW_EDGE && image.height() <= PREVIEW_EDGE {
        image
    } else {
        image.thumbnail(PREVIEW_EDGE, PREVIEW_EDGE)
    };

    fs::create_dir_all(&directory)
        .map_err(|err| format!("无法创建应用缓存目录：{err}"))?;
    let mut temporary = tempfile::Builder::new().prefix("preview-").suffix(".tmp")
        .tempfile_in(&directory)
        .map_err(|err| format!("无法写入临时缓存：{err}"))?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        resized.write_to(&mut writer, ImageFormat::Png)
            .map_err(|err| format!("无法编码缩略图：{err}"))?;
        // BufWriter::drop silently ignores errors. Flush explicitly before
        // publishing a visible PNG file to other GPUI image loaders.
        writer.flush().map_err(|err| format!("写入缩略图失败：{err}"))?;
    }
    // If the image changed while decoding, don't publish pixels under an old
    // size/mtime cache key. The next filesystem event/refresh can retry.
    let after = fs::metadata(path)
        .map_err(|err| format!("图片在预览生成期间无法访问：{err}"))?;
    if after.len() != metadata.len() ||
       after.modified().ok() != metadata.modified().ok() {
        return Err("图片正在修改，稍后重试或刷新图库".into());
    }
    // Avoid partial/overwritten files if two jobs raced on the same photo.
    match temporary.persist_noclobber(&output) {
        Ok(_) => {}
        Err(error) if output.is_file() => drop(error.file),
        Err(error) => return Err(format!("无法发布缩略图：{}", error.error)),
    }

    // Trim occasionally, not on each decode; old cached PNGs are discarded first.
    if WRITES.fetch_add(1, Ordering::Relaxed) % 32 == 0 {
        let _ = prune_cache(&directory, CACHE_BUDGET);
    }
    Ok(output)
}

pub fn prune_cache(directory: &Path, budget: u64) -> Result<usize, String> {
    if !directory.exists() {
        return Ok(0);
    }
    let mut records = Vec::new();
    let mut total = 0u64;
    for entry in fs::read_dir(directory).map_err(|err| err.to_string())?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "png") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() { continue }
        total = total.saturating_add(meta.len());
        records.push((meta.modified().unwrap_or(UNIX_EPOCH), path, meta.len()));
    }
    if total <= budget {
        return Ok(0);
    }
    records.sort_by_key(|(modified, _, _)| *modified);
    let mut deleted = 0;
    for (_, path, len) in records {
        if total <= budget { break }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(len);
            deleted += 1;
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn key_changes_after_content_edit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("image.png");
        fs::write(&file, b"first").unwrap();
        let before = cache_key(&file, &fs::metadata(&file).unwrap());
        fs::write(&file, b"this has more content").unwrap();
        let after = cache_key(&file, &fs::metadata(&file).unwrap());
        assert_ne!(before, after);
    }

    #[test]
    fn small_alpha_image_survives_thumbnail_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("tiny.png");
        let image = ImageBuffer::from_pixel(32, 16, Rgba([80u8, 120, 240, 95]));
        image.save(&original).unwrap();
        let output = generate_preview(&original).unwrap();
        let thumb = image::open(&output).unwrap().to_rgba8();
        assert_eq!(thumb.dimensions(), (32, 16));
        assert_eq!(thumb.get_pixel(0, 0).0, [80, 120, 240, 95]);
        assert_eq!(generate_preview(&original).unwrap(), output);
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn invalid_image_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("bad.png");
        fs::write(&original, b"corrupt").unwrap();
        assert!(generate_preview(&original).is_err());
    }

    #[test]
    fn budget_eviction_does_not_touch_other_formats() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.png"), vec![1; 24]).unwrap();
        fs::write(dir.path().join("b.png"), vec![2; 24]).unwrap();
        fs::write(dir.path().join("important.txt"), vec![3; 12]).unwrap();
        assert_eq!(prune_cache(dir.path(), 24).unwrap(), 1);
        assert!(dir.path().join("important.txt").exists());
    }
}
