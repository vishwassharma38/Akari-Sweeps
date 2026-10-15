//! Sprite discovery, validation, and pixel loading.
//!
//! Source frames are 256x320 RGBA PNGs in `assets/akari/sweep/`.
//! They are never rewritten; the TUI renders a scaled half-block
//! approximation (documented limitation) with transparency blended.

use std::path::{Path, PathBuf};

pub const SWEEP_WIDTH: u32 = 256;
pub const SWEEP_HEIGHT: u32 = 320;
pub const SWEEP_FRAMES: usize = 8;
/// Target 6 frames per second (≈166.7 ms per frame). All animation sets
/// share this timing policy through `Animator`.
pub const SWEEP_INTERVAL_MS: u64 = 167;

#[derive(Debug)]
pub enum SpriteError {
    Missing { index: usize, path: PathBuf },
    Load { path: PathBuf, reason: String },
    BadDimensions { path: PathBuf, w: u32, h: u32 },
}

impl std::fmt::Display for SpriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpriteError::Missing { index, path } => write!(
                f,
                "missing sprite frame {}: {} — restore assets/akari/sweep/akari_sweep_{:03}.png",
                index + 1,
                path.display(),
                index + 1
            ),
            SpriteError::Load { path, reason } => {
                write!(f, "cannot decode {}: {reason}", path.display())
            }
            SpriteError::BadDimensions { path, w, h } => write!(
                f,
                "{} is {w}x{h}, expected {}x{} — re-export at the documented size",
                path.display(),
                SWEEP_WIDTH,
                SWEEP_HEIGHT
            ),
        }
    }
}

/// One decoded RGBA frame (row-major, 4 bytes/px).
#[derive(Debug, Clone)]
pub struct SpriteFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Canonical file name for frame `index` (0-based).
pub fn sweep_frame_name(index: usize) -> String {
    format!("akari_sweep_{:03}.png", index + 1)
}

/// Candidate asset roots, in preference order: alongside the executable,
/// then the Cargo project dir (dev `cargo run`).
pub fn candidate_sweep_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("assets").join("akari").join("sweep"));
            // Installed layout may place assets next to the exe root.
            if let Some(parent) = dir.parent() {
                dirs.push(parent.join("assets").join("akari").join("sweep"));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("assets").join("akari").join("sweep"));
    }
    // Compile-time project dir as final fallback (dev only).
    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("akari")
            .join("sweep"),
    );
    dirs
}

/// Resolve the sweep asset directory (first candidate holding frame 001).
pub fn resolve_sweep_dir() -> Option<PathBuf> {
    for d in candidate_sweep_dirs() {
        if d.join(sweep_frame_name(0)).exists() {
            return Some(d);
        }
    }
    None
}

/// Load and validate all eight frames in order.
pub fn load_sweep_frames(dir: &Path) -> Result<Vec<SpriteFrame>, SpriteError> {
    let mut frames = Vec::with_capacity(SWEEP_FRAMES);
    for i in 0..SWEEP_FRAMES {
        let path = dir.join(sweep_frame_name(i));
        if !path.exists() {
            return Err(SpriteError::Missing { index: i, path });
        }
        let img = image::open(&path).map_err(|e| SpriteError::Load {
            path: path.clone(),
            reason: e.to_string(),
        })?;
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        if w != SWEEP_WIDTH || h != SWEEP_HEIGHT {
            return Err(SpriteError::BadDimensions { path, w, h });
        }
        frames.push(SpriteFrame {
            width: w,
            height: h,
            rgba: rgba.into_raw(),
        });
    }
    Ok(frames)
}

/// Best-effort load used by the TUI: `Ok(frames)` or a diagnostic string.
/// The TUI stays usable with a text placeholder when assets are missing.
pub fn load_sweep_frames_best_effort() -> Result<Vec<SpriteFrame>, String> {
    match resolve_sweep_dir() {
        Some(dir) => load_sweep_frames(&dir).map_err(|e| e.to_string()),
        None => Err("sprite assets not found: expected assets/akari/sweep/akari_sweep_001..008.png next to the executable or project root".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_names_sequential() {
        let names: Vec<_> = (0..8).map(sweep_frame_name).collect();
        assert_eq!(
            names,
            vec![
                "akari_sweep_001.png",
                "akari_sweep_002.png",
                "akari_sweep_003.png",
                "akari_sweep_004.png",
                "akari_sweep_005.png",
                "akari_sweep_006.png",
                "akari_sweep_007.png",
                "akari_sweep_008.png",
            ]
        );
    }

    #[test]
    fn real_assets_load_at_documented_size() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("akari")
            .join("sweep");
        if !dir.join(sweep_frame_name(0)).exists() {
            eprintln!("skipping: sweep assets not in tree");
            return;
        }
        let frames = load_sweep_frames(&dir).expect("sweep frames must decode");
        assert_eq!(frames.len(), 8);
        for f in &frames {
            assert_eq!((f.width, f.height), (256, 320));
        }
    }

    #[test]
    fn missing_frame_diagnostic() {
        let dir = std::env::temp_dir().join("akari-no-such-sweep-dir");
        let err = load_sweep_frames(&dir).unwrap_err();
        assert!(err.to_string().contains("missing sprite frame 1"));
    }

    #[test]
    fn bad_dimensions_rejected() {
        let dir = std::env::temp_dir().join(format!(
            "akari-bad-dim-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Write a tiny PNG via the image crate.
        let img = image::RgbaImage::new(4, 4);
        img.save(dir.join(sweep_frame_name(0))).unwrap();
        let err = load_sweep_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::BadDimensions { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
