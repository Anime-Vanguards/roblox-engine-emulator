//! EditableImage pixels offline: an image loaded from an asset or a file decoded to RGBA (assets
//! download into `<state dir>/images/<id>` the way keyframes do), and snapshots of images scripts
//! wrote, as PNG files under `<state dir>/editable-images/` that a recording's SurfaceAppearances
//! point at so ror draws what the script drew.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Cursor, ErrorKind},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use crate::{Engine, files};

/// An image's pixels, rows top to bottom, 4 bytes (RGBA) a pixel.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pixels of `uri` (an asset id as a client reads an image's, [`crate::content_id::asset`]:
/// `rbxassetid://N&version=V` is version V; or a local file: `file://<path>`, a path; a relative one
/// the project's, [`crate::Options::local_file`]). An asset downloads into `<state dir>/images/<id>`, a
/// pinned version into `<id>-v<version>`.
pub fn load(engine: &Engine, uri: &str) -> Result<Pixels, String> {
    files::note_unchecked("an image was read");
    let path = match engine.options.local_file(uri) {
        Some(path) => path,
        None => {
            let asset = crate::content_id::asset(uri).ok_or_else(|| format!("{uri}: not an asset id"))?;
            let cache = crate::state_dir(&engine.options.repo).join("images");
            let cached = cache.join(asset.file_stem());
            if !files::is_file(&cached) {
                crate::animation::source::fetch(engine, uri, asset, &cache, &cached)?;
            }
            cached
        }
    };
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    decode(&bytes).map_err(|error| format!("{uri}: {error}"))
}

/// A PNG's pixels as RGBA (every PNG colour type and depth).
pub fn decode(bytes: &[u8]) -> Result<Pixels, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| format!("not a PNG image ({error})"))?;
    let size = reader.output_buffer_size().ok_or("the image is too large")?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer).map_err(|error| error.to_string())?;
    let data = &buffer[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => return Err("an indexed PNG the decoder left unexpanded".into()),
    };
    Ok(Pixels { width: frame.width, height: frame.height, rgba })
}

static NEXT: AtomicU64 = AtomicU64::new(0);
static PRUNED: AtomicBool = AtomicBool::new(false);

/// Writes `rgba` (width × height pixels) as a new PNG snapshot and returns its path. Snapshots
/// older than a day are removed the first time a process writes one.
pub fn save(engine: &Engine, width: u32, height: u32, rgba: &[u8]) -> Result<PathBuf, String> {
    // (its name counts the process's snapshots: a run made ahead could not name it as the job would)
    files::note_unchecked("an image was written");
    let directory = crate::state_dir(&engine.options.repo).join("editable-images");
    if !PRUNED.swap(true, Ordering::Relaxed) {
        prune(&directory);
    }
    std::fs::create_dir_all(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(format!("{} bytes for a {width}x{height} image", rgba.len()));
    }
    let (path, file) = create_snapshot(&directory, std::process::id(), &NEXT)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(rgba).map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(path)
}

/// Allocates a new `{pid}-{index}.png` file without overwriting an existing snapshot left by an
/// earlier process that happened to share this PID (for example after Windows PID reuse).
fn create_snapshot(directory: &Path, pid: u32, next: &AtomicU64) -> Result<(PathBuf, File), String> {
    loop {
        let index = next.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!("{pid}-{index}.png"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
}

fn prune(directory: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        if entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|modified| modified < cutoff) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;

    #[test]
    fn save_does_not_overwrite_existing_snapshot_from_reused_pid() {
        let pid = std::process::id();
        let unique = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let root = std::env::temp_dir().join(format!("ree-images-pid-reuse-{pid}-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        let engine = Engine::new(Options { repo: root.clone(), ..Options::default() }).unwrap();
        let directory = crate::state_dir(&engine.options.repo).join("editable-images");
        std::fs::create_dir_all(&directory).unwrap();

        let start = NEXT.load(Ordering::Relaxed);
        let sentinel = b"existing snapshot from prior process";
        let mut preexisting = Vec::new();
        for index in start..start + 16 {
            let path = directory.join(format!("{pid}-{index}.png"));
            std::fs::write(&path, sentinel).unwrap();
            preexisting.push(path);
        }

        let rgba = [255, 0, 0, 255];
        let saved = save(&engine, 1, 1, &rgba).expect("save should pick a free snapshot name");
        let saved_bytes = std::fs::read(&saved).unwrap();
        let decoded = decode(&saved_bytes).unwrap();

        let intact = preexisting.iter().all(|p| std::fs::read(p).is_ok_and(|b| b == sentinel));
        let distinct = !preexisting.contains(&saved);

        for path in &preexisting {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(&saved);
        let _ = std::fs::remove_dir_all(&root);

        assert!(distinct, "save reused an existing snapshot path: {}", saved.display());
        assert!(intact, "save overwrote a pre-existing snapshot from a reused PID");
        assert_eq!((decoded.width, decoded.height, decoded.rgba), (1, 1, rgba.to_vec()));
    }

    #[test]
    fn create_snapshot_advances_past_colliding_indices() {
        let unique = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("ree-images-create-new-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("4242-0.png");
        let second = dir.join("4242-1.png");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();

        let next = AtomicU64::new(0);
        let (path, file) = create_snapshot(&dir, 4242, &next).unwrap();
        drop(file);
        let expected = dir.join("4242-2.png");
        let first_ok = std::fs::read(&first).unwrap() == b"first";
        let second_ok = std::fs::read(&second).unwrap() == b"second";
        let next_after = next.load(Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(path, expected);
        assert_eq!(next_after, 3);
        assert!(first_ok && second_ok);
    }
}
