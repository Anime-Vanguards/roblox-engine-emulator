//! EditableImage pixels offline: an image loaded from an asset or a file decoded to RGBA (assets
//! download into `<state dir>/images/<id>` the way keyframes do), and snapshots of images scripts
//! wrote, as PNG files under `<state dir>/editable-images/` that a recording's SurfaceAppearances
//! point at so ror draws what the script drew.

use std::{
    io::{BufWriter, Cursor},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use crate::{Engine, files};

/// An image's pixels, rows top to bottom, 4 bytes (RGBA) a pixel.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pixels of `uri` (`rbxassetid://N`, `N`, or a local file: `file://<path>`, a path; a relative
/// one the project's, [`crate::Options::local_file`]).
pub fn load(engine: &Engine, uri: &str) -> Result<Pixels, String> {
    files::note_unchecked("an image was read");
    let path = match engine.options.local_file(uri) {
        Some(path) => path,
        None => {
            let id = crate::animation::source::asset_id(uri).ok_or_else(|| format!("{uri}: not an asset id"))?;
            let cache = crate::state_dir(&engine.options.repo).join("images");
            let cached = cache.join(id.to_string());
            if !files::is_file(&cached) {
                crate::animation::source::fetch(engine, uri, &cache, &cached)?;
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

/// Writes `rgba` (width × height pixels) as a new PNG snapshot and returns its path. Snapshots
/// older than a day are removed the first time a process writes one.
pub fn save(engine: &Engine, width: u32, height: u32, rgba: &[u8]) -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // (its name counts the process's snapshots: a run made ahead could not name it as the job would)
    files::note_unchecked("an image was written");
    let directory = crate::state_dir(&engine.options.repo).join("editable-images");
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    if index == 0 {
        prune(&directory);
    }
    std::fs::create_dir_all(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(format!("{} bytes for a {width}x{height} image", rgba.len()));
    }
    let path = directory.join(format!("{}-{index}.png", std::process::id()));
    let file = std::fs::File::create(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(rgba).map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(path)
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
