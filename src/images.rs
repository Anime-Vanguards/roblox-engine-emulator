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

/// One pixel of an EditableImage draw: `source` (red, green, blue, alpha in 0-1) combined into `target` (bytes) by an
/// ImageCombineType, as a 0.747 client combines them (measured byte for byte, 120 pixels a type over 3 colour pairs, 8 source
/// and 5 target alphas: tools/parity/probes/editable_image_draw.luau). BlendSourceOver (the default) and AlphaBlend work in
/// single precision, each result cut down to a byte (`x * 255` truncated): BlendSourceOver's alpha is `sa + da * (1 - sa)`
/// and its colour the source's and the target's weighed by `sa` and `da * (1 - sa)` over that alpha (where that alpha is 0,
/// the target's colour, a solid draw's (`solid`: DrawRectangle, DrawCircle, DrawLine) the source's); AlphaBlend's every
/// channel, alpha too, `s * sa + d * (1 - sa)`; each sum's last term fused into it (`mul_add`: measured, one pixel a level
/// higher than a separate product gives); a shape's edge pixel at `coverage`. Add and Subtract work in
/// bytes: each colour channel the target's plus or minus the source's times its alpha over 255 (truncated), the alpha the
/// target's plus or minus the source's, clamped. Multiply: every channel `s * d / 255` rounded up. Overwrite: the source.
pub fn combine(kind: &str, source: [f32; 4], target: [u8; 4], solid: bool, coverage: f32) -> [u8; 4] {
    let byte = |x: f32| (x * 255.0).clamp(0.0, 255.0) as u8;
    let unit = |b: u8| f32::from(b) / 255.0;
    // (a shape's edge pixel, covered in part: Overwrite mixes the pixel toward the source by the share, the others take the
    // source's alpha times it; measured in a 0.747 client on circles)
    if coverage < 1.0 {
        if kind == "Overwrite" {
            return [0, 1, 2, 3].map(|i| byte(unit(target[i]) + (source[i] - unit(target[i])) * coverage));
        }
        return combine(kind, [source[0], source[1], source[2], source[3] * coverage], target, solid, 1.0);
    }
    let d = target.map(unit);
    let (sa, da) = (source[3], d[3]);
    // (the byte modes read the source back as the bytes it was made from)
    let source_bytes = || source.map(|x| (x * 255.0).round().clamp(0.0, 255.0) as u8);
    match kind {
        "Overwrite" => source_bytes(),
        "AlphaBlend" => {
            let inverse = 1.0 - sa;
            [0, 1, 2, 3].map(|i| byte(d[i].mul_add(inverse, source[i] * sa)))
        }
        "Add" | "Subtract" => {
            let s = source_bytes();
            let sign = if kind == "Add" { 1 } else { -1 };
            let colour = |i: usize| (i32::from(target[i]) + sign * (i32::from(s[i]) * i32::from(s[3]) / 255)).clamp(0, 255) as u8;
            [colour(0), colour(1), colour(2), (i32::from(target[3]) + sign * i32::from(s[3])).clamp(0, 255) as u8]
        }
        "Multiply" => {
            let s = source_bytes();
            [0, 1, 2, 3].map(|i| ((u32::from(s[i]) * u32::from(target[i]) + 254) / 255) as u8)
        }
        _ => {
            let inverse = 1.0 - sa;
            let alpha = da.mul_add(inverse, sa);
            if alpha <= 0.0 {
                let kept = if solid { source_bytes() } else { target };
                return [kept[0], kept[1], kept[2], 0];
            }
            let colour = |i: usize| byte((d[i] * da).mul_add(inverse, source[i] * sa) / alpha);
            [colour(0), colour(1), colour(2), byte(alpha)]
        }
    }
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

    /// The combine types byte for byte as a 0.747 client combines them (tools/parity/probes/editable_image_draw.luau: a few of
    /// its 600 pixels, the fused sum's included).
    #[test]
    fn combine_types_follow_a_client() {
        let unit = |pixel: [u8; 4]| pixel.map(|b| f32::from(b) / 255.0);
        let cases: [(&str, [u8; 4], [u8; 4], [u8; 4]); 8] = [
            ("BlendSourceOver", [200, 50, 25, 64], [10, 100, 230, 64], [118, 71, 112, 111]),
            ("BlendSourceOver", [200, 50, 25, 0], [10, 100, 230, 0], [10, 100, 230, 0]),
            ("AlphaBlend", [200, 50, 25, 64], [10, 100, 230, 128], [57, 87, 178, 111]),
            ("AlphaBlend", [200, 50, 25, 153], [10, 100, 200, 160], [124, 70, 94, 155]),
            ("Add", [200, 50, 25, 64], [10, 100, 230, 255], [60, 112, 236, 255]),
            ("Subtract", [200, 50, 25, 1], [10, 100, 230, 1], [10, 100, 230, 0]),
            ("Multiply", [200, 50, 25, 64], [10, 100, 230, 64], [8, 20, 23, 17]),
            ("Overwrite", [200, 50, 25, 0], [10, 100, 230, 255], [200, 50, 25, 0]),
        ];
        for (kind, source, target, want) in cases {
            assert_eq!(combine(kind, unit(source), target, false, 1.0), want, "{kind} {source:?} over {target:?}");
        }
        // a solid draw keeps its own colour where the alpha comes out 0, and an edge pixel's share mixes Overwrite
        assert_eq!(combine("BlendSourceOver", unit([204, 51, 25, 0]), [10, 100, 230, 0], true, 1.0), [204, 51, 25, 0]);
        assert_eq!(combine("BlendSourceOver", [1.0, 1.0, 1.0, 1.0], [0, 0, 0, 0], true, 0.625), [255, 255, 255, 159]);
    }

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
