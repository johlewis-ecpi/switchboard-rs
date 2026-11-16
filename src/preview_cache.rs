use crate::{
    services::Preview,
    store::{atomic_write, read_bounded},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::UNIX_EPOCH,
};

// Serialize requests for the same file without holding up all other previews.
static LOCKS: [Mutex<()>; 32] = [const { Mutex::new(()) }; 32];
const MAX_CACHE_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct CachedPreview {
    signature: String,
    preview: Preview,
}

fn directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Library/Caches/Switchboard/Thumbnails")
}

// Metadata only: looking up an existing thumbnail must not open the document or
// ask a file provider to download its contents. In particular, don't include
// dataless flags or change time: eviction can change those without editing it.
fn signature(path: &Path, variant: &str) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let (before_epoch, duration) = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => (false, duration),
        Err(error) => (true, error.duration()),
    };
    let mut hash = Sha256::new();
    hash.update(b"switchboard-thumbnail-cache-v1\0");
    hash.update(variant.as_bytes());
    // Renderer changes automatically invalidate thumbnails from older builds.
    hash.update(include_str!("../native/Preview.swift").as_bytes());
    hash.update([u8::from(before_epoch)]);
    hash.update(duration.as_secs().to_le_bytes());
    hash.update(duration.subsec_nanos().to_le_bytes());
    hash.update(metadata.len().to_le_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hash.update(metadata.dev().to_le_bytes());
        hash.update(metadata.ino().to_le_bytes());
    }
    Some(format!("{:x}", hash.finalize()))
}

fn successful(preview: &Preview) -> bool {
    if preview.error.is_some() {
        return false;
    }
    if let Some(image) = &preview.image {
        use base64::Engine;
        return image
            .strip_prefix("data:image/png;base64,")
            .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
            .is_some_and(|bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    preview.text.is_some()
}

pub(crate) fn get_or_render(
    path: &Path,
    variant: &str,
    render: impl FnOnce() -> Preview,
) -> Preview {
    get_or_render_in(&directory(), path, variant, render)
}

fn get_or_render_in(
    directory: &Path,
    path: &Path,
    variant: &str,
    render: impl FnOnce() -> Preview,
) -> Preview {
    let mut hash = Sha256::new();
    hash.update(variant.as_bytes());
    hash.update([0]);
    hash.update(path.as_os_str().as_encoded_bytes());
    let key = hash.finalize();
    let _guard = LOCKS[key[0] as usize % LOCKS.len()]
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let file = directory.join(format!("{key:x}.json"));
    let before = signature(path, variant);
    if let Some(signature) = &before
        && let Ok(bytes) = read_bounded(&file, MAX_CACHE_BYTES)
        && let Ok(cached) = serde_json::from_slice::<CachedPreview>(&bytes)
        && cached.signature == *signature
        && successful(&cached.preview)
    {
        return cached.preview;
    }

    let preview = render();
    if let Some(signature) = before
        && successful(&preview)
        && self::signature(path, variant).as_ref() == Some(&signature)
        && let Ok(bytes) = serde_json::to_vec(&CachedPreview {
            signature,
            preview: preview.clone(),
        })
        && bytes.len() as u64 <= MAX_CACHE_BYTES
    {
        // An unwritable or full cache must never prevent a working preview.
        let _ = atomic_write(&file, &bytes);
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(text: &str) -> Preview {
        Preview {
            text: Some(text.into()),
            image: None,
            error: None,
            viewer: None,
        }
    }

    #[test]
    fn unchanged_file_reuses_disk_cache_without_renderer_state() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("document.docx");
        let cache = temp.path().join("cache");
        fs::write(&source, "document").unwrap();
        let image = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl1sAAAAASUVORK5CYII=";
        let first = get_or_render_in(&cache, &source, "preview", || Preview {
            image: Some(image.into()),
            ..Preview::default()
        });
        assert_eq!(first.image.as_deref(), Some(image));
        // A separate invocation uses only persisted bytes, as on relaunch.
        let cached = get_or_render_in(&cache, &source, "preview", || {
            panic!("unchanged file was reparsed")
        });
        assert_eq!(cached.image.as_deref(), Some(image));
    }

    #[test]
    fn concurrent_requests_render_a_file_once() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("document");
        let cache = temp.path().join("cache");
        fs::write(&source, "document").unwrap();
        let start = Barrier::new(2);
        let renders = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    start.wait();
                    let result = get_or_render_in(&cache, &source, "preview", || {
                        renders.fetch_add(1, Ordering::SeqCst);
                        rendered("shared preview")
                    });
                    assert_eq!(result.text.as_deref(), Some("shared preview"));
                });
            }
        });
        assert_eq!(renders.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn edits_replacements_and_renderer_variants_invalidate_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("document");
        let cache = temp.path().join("cache");
        fs::write(&source, "first").unwrap();
        get_or_render_in(&cache, &source, "preview", || rendered("first"));
        // Same-size edit with a different modification timestamp.
        let modified = fs::metadata(&source).unwrap().modified().unwrap();
        fs::write(&source, "other").unwrap();
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(modified + std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            get_or_render_in(&cache, &source, "preview", || rendered("edited"))
                .text
                .as_deref(),
            Some("edited")
        );
        let modified = fs::metadata(&source).unwrap().modified().unwrap();
        let replacement = temp.path().join("replacement");
        fs::write(&replacement, "third").unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        fs::rename(replacement, &source).unwrap();
        assert_eq!(
            get_or_render_in(&cache, &source, "preview", || rendered("replaced"))
                .text
                .as_deref(),
            Some("replaced")
        );
        assert_eq!(
            get_or_render_in(&cache, &source, "icon", || rendered("icon"))
                .text
                .as_deref(),
            Some("icon")
        );
    }

    #[test]
    fn failed_or_corrupt_previews_are_retried_and_unwritable_cache_is_optional() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("document");
        let cache = temp.path().join("cache");
        fs::write(&source, "document").unwrap();
        let failed = get_or_render_in(&cache, &source, "preview", || Preview {
            error: Some("not readable yet".into()),
            text: None,
            image: None,
            viewer: None,
        });
        assert!(failed.error.is_some());
        assert!(!cache.exists());
        get_or_render_in(&cache, &source, "preview", || rendered("recovered"));
        let file = fs::read_dir(&cache)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::write(&file, "truncated cache").unwrap();
        assert_eq!(
            get_or_render_in(&cache, &source, "preview", || rendered("rebuilt"))
                .text
                .as_deref(),
            Some("rebuilt")
        );
        let blocked = temp.path().join("not-a-directory");
        fs::write(&blocked, "occupied").unwrap();
        assert_eq!(
            get_or_render_in(&blocked, &source, "preview", || rendered("uncached"))
                .text
                .as_deref(),
            Some("uncached")
        );
    }

    #[test]
    fn changes_during_render_are_not_saved_and_missing_sources_never_reuse_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("document");
        let cache = temp.path().join("cache");
        fs::write(&source, "first").unwrap();
        get_or_render_in(&cache, &source, "preview", || {
            fs::write(&source, "changed during rendering").unwrap();
            rendered("stale")
        });
        assert!(!cache.exists());
        get_or_render_in(&cache, &source, "preview", || rendered("current"));
        fs::remove_file(&source).unwrap();
        let result = get_or_render_in(&cache, &source, "preview", || Preview {
            error: Some("missing".into()),
            text: None,
            image: None,
            viewer: None,
        });
        assert_eq!(result.error.as_deref(), Some("missing"));
    }
}
