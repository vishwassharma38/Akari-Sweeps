//! Sprite discovery, normalization, and pixel loading.
//!
//! Each animation set is a sequence of PNGs in its own
//! `assets/akari/<set>/` directory. Replacement frames may use any
//! dimensions: the loader decodes every expected frame, derives one shared
//! canvas from the largest width and height in the set, and centers each
//! source image on a transparent canvas of that size. Frames are never
//! rewritten; the TUI renders a scaled half-block approximation (documented
//! limitation) with transparent pixels blended over the terminal background.

use std::path::{Path, PathBuf};

pub const SWEEP_FRAMES: usize = 8;
pub const IDLE_FRAMES: usize = 8;
pub const HEALTH_FRAMES: usize = 8;
/// Four unique poses. The asset folder also contains exact duplicates as
/// files 005–008; the loader follows the canonical 001–004 sequence.
pub const ABOUT_FRAMES: usize = 4;
/// Four unique poses. The asset folder also contains exact duplicates as
/// files 005–008; the loader follows the canonical 001–004 sequence.
pub const HELP_FRAMES: usize = 4;
/// Target 6 frames per second (≈166.7 ms per frame). All animation sets
/// share this timing policy through `Animator`.
pub const SWEEP_INTERVAL_MS: u64 = 167;

/// Safety budget for the normalized RGBA data of one animation set (all
/// frames). This is deliberately far above any sensible terminal sprite and
/// is a resource guard, not an expected image size: replacement art may use
/// any dimensions or aspect ratio within it.
pub const MAX_ANIMATION_BYTES: usize = 256 * 1024 * 1024;

/// Filesystem identity and loading constraints for one animation set.
/// Additional sets add a constant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSetSpec {
    /// Asset directory under `assets/akari/` (may be nested).
    pub dir: &'static str,
    /// File-name prefix before `_NNN.png`.
    pub prefix: &'static str,
    pub frame_count: usize,
}

impl SpriteSetSpec {
    pub const SWEEP: Self = Self {
        dir: "sweep",
        prefix: "akari_sweep",
        frame_count: SWEEP_FRAMES,
    };

    pub const IDLE: Self = Self {
        dir: "idle",
        prefix: "idle",
        frame_count: IDLE_FRAMES,
    };

    pub const HEALTH: Self = Self {
        dir: "health",
        prefix: "health",
        frame_count: HEALTH_FRAMES,
    };

    pub const ABOUT: Self = Self {
        dir: "about",
        prefix: "about",
        frame_count: ABOUT_FRAMES,
    };

    pub const HELP: Self = Self {
        dir: "help",
        prefix: "help",
        frame_count: HELP_FRAMES,
    };

    /// Canonical file name for frame `index` (0-based).
    pub fn frame_name(self, index: usize) -> String {
        format!("{}_{:03}.png", self.prefix, index + 1)
    }

    /// Diagnostic hint naming the expected asset directory and file range.
    pub fn asset_hint(self) -> String {
        format!(
            "assets/akari/{}/{}_001..{:03}.png",
            self.dir, self.prefix, self.frame_count
        )
    }

    /// Candidate asset roots, in preference order: alongside the
    /// executable, then the Cargo project dir (dev `cargo run`).
    pub fn candidate_dirs(self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                dirs.push(dir.join("assets").join("akari").join(self.dir));
                // Installed layout may place assets next to the exe root.
                if let Some(parent) = dir.parent() {
                    dirs.push(parent.join("assets").join("akari").join(self.dir));
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd.join("assets").join("akari").join(self.dir));
        }
        // Compile-time project dir as final fallback (dev only).
        dirs.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join("akari")
                .join(self.dir),
        );
        dirs
    }

    /// Resolve this asset directory (first candidate holding frame 001).
    pub fn resolve_dir(self) -> Option<PathBuf> {
        self.candidate_dirs()
            .into_iter()
            .find(|d| d.join(self.frame_name(0)).exists())
    }

    /// Load every expected frame in order and normalize the set onto one
    /// shared canvas sized to the largest source width/height. Every source
    /// image is centered, unmodified, on transparent padding.
    pub fn load_frames(self, dir: &Path) -> Result<Vec<SpriteFrame>, SpriteError> {
        // Pass 1: resolve paths and read header dimensions without keeping
        // full pixel buffers. This lets the shared canvas be sized before
        // any unbounded RGBA allocation happens.
        let mut sources = Vec::with_capacity(self.frame_count);
        for i in 0..self.frame_count {
            let path = dir.join(self.frame_name(i));
            if !path.exists() {
                return Err(SpriteError::Missing {
                    spec: self,
                    index: i,
                    path,
                });
            }
            let mut reader = image::ImageReader::open(&path).map_err(|e| load_error(&path, e))?;
            reader.limits(decode_limits());
            let (width, height) = reader.into_dimensions().map_err(|e| load_error(&path, e))?;
            sources.push((path, width, height));
        }

        let canvas_width = sources
            .iter()
            .map(|(_, width, _)| *width)
            .max()
            .unwrap_or(0);
        let canvas_height = sources
            .iter()
            .map(|(_, _, height)| *height)
            .max()
            .unwrap_or(0);
        let canvas_len = checked_canvas_len(self, canvas_width, canvas_height)?;

        // Pass 2: decode one frame at a time and copy it into its centered
        // canvas position. Pixels (including alpha) are copied verbatim.
        let mut frames = Vec::with_capacity(self.frame_count);
        for (path, source_width, source_height) in sources {
            let mut reader = image::ImageReader::open(&path).map_err(|e| load_error(&path, e))?;
            reader.limits(decode_limits());
            let rgba = reader
                .decode()
                .map_err(|e| load_error(&path, e))?
                .to_rgba8();
            if rgba.width() != source_width || rgba.height() != source_height {
                return Err(SpriteError::Load {
                    path: path.clone(),
                    reason: format!(
                        "decoded dimensions {}x{} do not match the PNG header {source_width}x{source_height}",
                        rgba.width(),
                        rgba.height()
                    ),
                });
            }

            let offset_x = (canvas_width - source_width) / 2;
            let offset_y = (canvas_height - source_height) / 2;
            let mut canvas = Vec::new();
            canvas
                .try_reserve_exact(canvas_len)
                .map_err(|_| SpriteError::CanvasTooLarge {
                    spec: self,
                    width: canvas_width,
                    height: canvas_height,
                    limit: MAX_ANIMATION_BYTES,
                })?;
            canvas.resize(canvas_len, 0);

            let source_row_bytes = source_width as usize * 4;
            let canvas_row_bytes = canvas_width as usize * 4;
            let raw = rgba.as_raw();
            for row in 0..source_height as usize {
                let src_start = row * source_row_bytes;
                let dst_start =
                    (offset_y as usize + row) * canvas_row_bytes + offset_x as usize * 4;
                canvas[dst_start..dst_start + source_row_bytes]
                    .copy_from_slice(&raw[src_start..src_start + source_row_bytes]);
            }

            frames.push(SpriteFrame {
                source_width,
                source_height,
                canvas_width,
                canvas_height,
                rgba: canvas,
            });
        }
        Ok(frames)
    }

    /// Best-effort load used by the TUI: `Ok(frames)` or a diagnostic
    /// string. The TUI stays usable with a text placeholder when assets
    /// are missing.
    pub fn load_frames_best_effort(self) -> Result<Vec<SpriteFrame>, String> {
        match self.resolve_dir() {
            Some(dir) => self.load_frames(&dir).map_err(|e| e.to_string()),
            None => Err(format!(
                "sprite assets not found: expected {} next to the executable or project root",
                self.asset_hint()
            )),
        }
    }
}

/// Decoder limits: no fixed width/height requirement, but a single decode
/// may not allocate more than one animation's safety budget.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ANIMATION_BYTES as u64);
    limits
}

/// Validate the shared canvas size and return its RGBA byte length.
///
/// Checked arithmetic prevents overflow for hostile dimensions; the budget
/// keeps memory use bounded without reintroducing a fixed asset size.
fn checked_canvas_len(spec: SpriteSetSpec, width: u32, height: u32) -> Result<usize, SpriteError> {
    let total_bytes = (width as u128)
        .checked_mul(height as u128)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| bytes.checked_mul(spec.frame_count as u128))
        .unwrap_or(u128::MAX);
    let per_canvas = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    match per_canvas {
        Some(bytes) if total_bytes <= MAX_ANIMATION_BYTES as u128 => Ok(bytes),
        _ => Err(SpriteError::CanvasTooLarge {
            spec,
            width,
            height,
            limit: MAX_ANIMATION_BYTES,
        }),
    }
}

fn load_error(path: &Path, error: impl std::fmt::Display) -> SpriteError {
    SpriteError::Load {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

#[derive(Debug)]
pub enum SpriteError {
    Missing {
        spec: SpriteSetSpec,
        index: usize,
        path: PathBuf,
    },
    Load {
        path: PathBuf,
        reason: String,
    },
    CanvasTooLarge {
        spec: SpriteSetSpec,
        width: u32,
        height: u32,
        limit: usize,
    },
}

impl std::fmt::Display for SpriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpriteError::Missing { spec, index, path } => write!(
                f,
                "missing sprite frame {} for {} set: {} — restore {}",
                index + 1,
                spec.dir,
                path.display(),
                spec.asset_hint()
            ),
            SpriteError::Load { path, reason } => {
                write!(f, "cannot decode {}: {reason}", path.display())
            }
            SpriteError::CanvasTooLarge {
                spec,
                width,
                height,
                limit,
            } => write!(
                f,
                "cannot normalize the {} animation: shared {width}x{height} canvas for {} frames \
                 exceeds the {} MiB safety budget — provide smaller PNGs",
                spec.dir,
                spec.frame_count,
                limit / (1024 * 1024)
            ),
        }
    }
}

/// One decoded frame normalized onto its animation set's shared RGBA canvas
/// (row-major, 4 bytes/px). `source_*` preserves the original PNG dimensions
/// for accurate reporting; `canvas_*` is identical for every frame in a set.
#[derive(Debug, Clone)]
pub struct SpriteFrame {
    /// Original PNG width in pixels.
    pub source_width: u32,
    /// Original PNG height in pixels.
    pub source_height: u32,
    /// Shared canvas width (largest source width in the set).
    pub canvas_width: u32,
    /// Shared canvas height (largest source height in the set).
    pub canvas_height: u32,
    /// Normalized canvas pixels: `canvas_width × canvas_height`, RGBA.
    pub rgba: Vec<u8>,
}

impl SpriteFrame {
    /// Top-left pixel of the original image inside the shared canvas.
    #[allow(dead_code)]
    pub fn source_offset(&self) -> (u32, u32) {
        (
            self.canvas_width.saturating_sub(self.source_width) / 2,
            self.canvas_height.saturating_sub(self.source_height) / 2,
        )
    }
}

/// Canonical sweep file name for frame `index` (0-based).
#[allow(dead_code)]
pub fn sweep_frame_name(index: usize) -> String {
    SpriteSetSpec::SWEEP.frame_name(index)
}

/// Candidate sweep asset roots, in preference order.
#[allow(dead_code)]
pub fn candidate_sweep_dirs() -> Vec<PathBuf> {
    SpriteSetSpec::SWEEP.candidate_dirs()
}

/// Resolve the sweep asset directory (first candidate holding frame 001).
#[allow(dead_code)]
pub fn resolve_sweep_dir() -> Option<PathBuf> {
    SpriteSetSpec::SWEEP.resolve_dir()
}

/// Load and normalize all eight sweep frames in order.
#[allow(dead_code)]
pub fn load_sweep_frames(dir: &Path) -> Result<Vec<SpriteFrame>, SpriteError> {
    SpriteSetSpec::SWEEP.load_frames(dir)
}

/// Best-effort sweep load used by the TUI.
#[allow(dead_code)]
pub fn load_sweep_frames_best_effort() -> Result<Vec<SpriteFrame>, String> {
    SpriteSetSpec::SWEEP.load_frames_best_effort()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "akari-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest_dir(spec: SpriteSetSpec) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("akari")
            .join(spec.dir)
    }

    fn save_solid(dir: &Path, spec: SpriteSetSpec, index: usize, w: u32, h: u32, rgba: [u8; 4]) {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        img.save(dir.join(spec.frame_name(index))).unwrap();
    }

    fn source_image(spec: SpriteSetSpec, dir: &Path, index: usize) -> image::RgbaImage {
        image::open(dir.join(spec.frame_name(index)))
            .expect("source frame must decode")
            .to_rgba8()
    }

    fn canvas_pixel(frame: &SpriteFrame, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y as usize * frame.canvas_width as usize) + x as usize) * 4;
        [
            frame.rgba[idx],
            frame.rgba[idx + 1],
            frame.rgba[idx + 2],
            frame.rgba[idx + 3],
        ]
    }

    /// The source pixels must appear verbatim at the frame's centered
    /// offset in the shared canvas.
    fn assert_source_preserved(frame: &SpriteFrame, source: &image::RgbaImage) {
        assert_eq!(
            (frame.source_width, frame.source_height),
            (source.width(), source.height())
        );
        let (offset_x, offset_y) = frame.source_offset();
        let source_row_bytes = source.width() as usize * 4;
        let canvas_row_bytes = frame.canvas_width as usize * 4;
        let raw = source.as_raw();
        for row in 0..source.height() as usize {
            let src_start = row * source_row_bytes;
            let dst_start = (offset_y as usize + row) * canvas_row_bytes + offset_x as usize * 4;
            assert_eq!(
                &frame.rgba[dst_start..dst_start + source_row_bytes],
                &raw[src_start..src_start + source_row_bytes],
                "source row {row} must be preserved in the canvas"
            );
        }
    }

    #[test]
    fn frame_names_sequential_for_all_sets() {
        let sweep: Vec<_> = (0..SWEEP_FRAMES)
            .map(|i| SpriteSetSpec::SWEEP.frame_name(i))
            .collect();
        assert_eq!(
            sweep,
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
        let idle: Vec<_> = (0..IDLE_FRAMES)
            .map(|i| SpriteSetSpec::IDLE.frame_name(i))
            .collect();
        assert_eq!(
            idle,
            vec![
                "idle_001.png",
                "idle_002.png",
                "idle_003.png",
                "idle_004.png",
                "idle_005.png",
                "idle_006.png",
                "idle_007.png",
                "idle_008.png",
            ]
        );
        let health: Vec<_> = (0..HEALTH_FRAMES)
            .map(|i| SpriteSetSpec::HEALTH.frame_name(i))
            .collect();
        assert_eq!(
            health,
            vec![
                "health_001.png",
                "health_002.png",
                "health_003.png",
                "health_004.png",
                "health_005.png",
                "health_006.png",
                "health_007.png",
                "health_008.png",
            ]
        );
        let about: Vec<_> = (0..ABOUT_FRAMES)
            .map(|i| SpriteSetSpec::ABOUT.frame_name(i))
            .collect();
        assert_eq!(
            about,
            vec![
                "about_001.png",
                "about_002.png",
                "about_003.png",
                "about_004.png",
            ]
        );
        let help: Vec<_> = (0..HELP_FRAMES)
            .map(|i| SpriteSetSpec::HELP.frame_name(i))
            .collect();
        assert_eq!(
            help,
            vec![
                "help_001.png",
                "help_002.png",
                "help_003.png",
                "help_004.png",
            ]
        );
    }

    #[test]
    fn every_shipped_set_resolves_and_loads_its_frames() {
        for spec in [
            SpriteSetSpec::SWEEP,
            SpriteSetSpec::IDLE,
            SpriteSetSpec::HEALTH,
            SpriteSetSpec::ABOUT,
            SpriteSetSpec::HELP,
        ] {
            let dir = manifest_dir(spec);
            assert!(
                dir.join(spec.frame_name(0)).exists(),
                "missing shipped assets: {}",
                spec.asset_hint()
            );
            let resolved = spec
                .resolve_dir()
                .unwrap_or_else(|| panic!("{} assets must be discovered", spec.dir));
            assert!(
                resolved.ends_with(Path::new("assets").join("akari").join(spec.dir)),
                "resolved {} to {}",
                spec.dir,
                resolved.display()
            );
            let frames = spec.load_frames(&resolved).expect("frames must decode");
            assert_eq!(frames.len(), spec.frame_count);

            let max_width = frames.iter().map(|f| f.source_width).max().unwrap();
            let max_height = frames.iter().map(|f| f.source_height).max().unwrap();
            for (i, frame) in frames.iter().enumerate() {
                assert_eq!(
                    (frame.canvas_width, frame.canvas_height),
                    (max_width, max_height),
                    "{} frame {i} must share the set canvas",
                    spec.dir
                );
                assert_source_preserved(frame, &source_image(spec, &dir, i));
            }
        }
    }

    #[test]
    fn real_assets_load_with_shared_canvases() {
        for spec in [
            SpriteSetSpec::SWEEP,
            SpriteSetSpec::IDLE,
            SpriteSetSpec::HEALTH,
            SpriteSetSpec::ABOUT,
            SpriteSetSpec::HELP,
        ] {
            let dir = manifest_dir(spec);
            if !dir.join(spec.frame_name(0)).exists() {
                eprintln!("skipping: {} assets not in tree", spec.dir);
                continue;
            }
            let frames = spec.load_frames(&dir).expect("frames must decode");
            assert_eq!(frames.len(), spec.frame_count);
            let canvas = (frames[0].canvas_width, frames[0].canvas_height);
            for frame in &frames {
                assert_eq!((frame.canvas_width, frame.canvas_height), canvas);
                assert_eq!(
                    frame.rgba.len(),
                    (canvas.0 as usize) * (canvas.1 as usize) * 4
                );
            }
        }
    }

    #[test]
    fn idle_frames_are_loaded_in_order() {
        let spec = SpriteSetSpec::IDLE;
        let dir = manifest_dir(spec);
        if !dir.join(spec.frame_name(0)).exists() {
            eprintln!("skipping: idle assets not in tree");
            return;
        }
        let frames = spec.load_frames(&dir).expect("idle frames must decode");
        for (i, frame) in frames.iter().enumerate() {
            assert_source_preserved(frame, &source_image(spec, &dir, i));
        }
    }

    #[test]
    fn idle_dir_is_discovered() {
        let spec = SpriteSetSpec::IDLE;
        if !manifest_dir(spec).join(spec.frame_name(0)).exists() {
            eprintln!("skipping: idle assets not in tree");
            return;
        }
        let dir = spec.resolve_dir().expect("idle assets must be discovered");
        assert!(dir.ends_with(Path::new("assets").join("akari").join("idle")));
    }

    #[test]
    fn health_frames_are_loaded_in_order() {
        let spec = SpriteSetSpec::HEALTH;
        let dir = manifest_dir(spec);
        if !dir.join(spec.frame_name(0)).exists() {
            eprintln!("skipping: health assets not in tree");
            return;
        }
        let frames = spec.load_frames(&dir).expect("health frames must decode");
        assert_eq!(frames.len(), HEALTH_FRAMES);
        for (i, frame) in frames.iter().enumerate() {
            assert_source_preserved(frame, &source_image(spec, &dir, i));
        }
    }

    #[test]
    fn health_dir_is_discovered() {
        let spec = SpriteSetSpec::HEALTH;
        if !manifest_dir(spec).join(spec.frame_name(0)).exists() {
            eprintln!("skipping: health assets not in tree");
            return;
        }
        let dir = spec
            .resolve_dir()
            .expect("health assets must be discovered");
        assert!(
            dir.ends_with(Path::new("assets").join("akari").join("health")),
            "resolved {}",
            dir.display()
        );
    }

    #[test]
    fn about_frames_are_loaded_in_order() {
        let spec = SpriteSetSpec::ABOUT;
        let dir = manifest_dir(spec);
        if !dir.join(spec.frame_name(0)).exists() {
            eprintln!("skipping: about assets not in tree");
            return;
        }
        let frames = spec.load_frames(&dir).expect("about frames must decode");
        assert_eq!(frames.len(), ABOUT_FRAMES);
        for (i, frame) in frames.iter().enumerate() {
            assert_source_preserved(frame, &source_image(spec, &dir, i));
        }
    }

    #[test]
    fn about_dir_is_discovered() {
        let spec = SpriteSetSpec::ABOUT;
        if !manifest_dir(spec).join(spec.frame_name(0)).exists() {
            eprintln!("skipping: about assets not in tree");
            return;
        }
        let dir = spec.resolve_dir().expect("about assets must be discovered");
        assert!(
            dir.ends_with(Path::new("assets").join("akari").join("about")),
            "resolved {}",
            dir.display()
        );
    }

    #[test]
    fn help_frames_are_loaded_in_order() {
        let spec = SpriteSetSpec::HELP;
        let dir = manifest_dir(spec);
        if !dir.join(spec.frame_name(0)).exists() {
            eprintln!("skipping: help assets not in tree");
            return;
        }
        let frames = spec.load_frames(&dir).expect("help frames must decode");
        assert_eq!(frames.len(), HELP_FRAMES);
        for (i, frame) in frames.iter().enumerate() {
            assert_source_preserved(frame, &source_image(spec, &dir, i));
        }
    }

    #[test]
    fn help_dir_is_discovered() {
        let spec = SpriteSetSpec::HELP;
        if !manifest_dir(spec).join(spec.frame_name(0)).exists() {
            eprintln!("skipping: help assets not in tree");
            return;
        }
        let dir = spec.resolve_dir().expect("help assets must be discovered");
        assert!(
            dir.ends_with(Path::new("assets").join("akari").join("help")),
            "resolved {}",
            dir.display()
        );
    }

    #[test]
    fn arbitrary_dimensions_load_successfully() {
        let spec = SpriteSetSpec::HELP;
        let dir = temp_dir("arbitrary-dims");
        let sizes = [(11, 7), (3, 19), (42, 5), (1, 1)];
        for (i, (w, h)) in sizes.iter().enumerate() {
            save_solid(&dir, spec, i, *w, *h, [10 + i as u8, 20, 30, 255]);
        }
        let frames = spec.load_frames(&dir).expect("arbitrary PNGs must load");
        assert_eq!(frames.len(), sizes.len());
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(
                (frame.source_width, frame.source_height),
                sizes[i],
                "source dimensions must be preserved for frame {i}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mixed_help_frames_share_a_centered_transparent_canvas() {
        let spec = SpriteSetSpec::HELP;
        let dir = temp_dir("mixed-help");
        // Regression scenario: Help frames differ in dimensions, including
        // 360x452 and 346x557.
        let sizes = [(360, 452), (346, 557), (100, 200), (360, 557)];
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        for i in 0..sizes.len() {
            save_solid(&dir, spec, i, sizes[i].0, sizes[i].1, colors[i]);
        }

        let frames = spec.load_frames(&dir).expect("mixed Help frames must load");
        assert_eq!(frames.len(), sizes.len());
        for frame in &frames {
            assert_eq!(
                (frame.canvas_width, frame.canvas_height),
                (360, 557),
                "canvas must use the largest required dimensions"
            );
            assert_eq!(
                frame.rgba.len(),
                (360 * 557 * 4) as usize,
                "every frame must carry the full shared canvas"
            );
        }
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(
                (frame.source_width, frame.source_height),
                sizes[i],
                "frame {i} must retain its original dimensions"
            );
            let (offset_x, offset_y) = frame.source_offset();
            assert_eq!(
                canvas_pixel(frame, offset_x, offset_y),
                colors[i],
                "frame {i} must be centered with its pixels preserved"
            );
        }
        assert_eq!(frames[0].source_offset(), (0, 52));
        assert_eq!(frames[1].source_offset(), (7, 0));
        // Transparent padding, not duplicated edge pixels or black fill.
        assert_eq!(canvas_pixel(&frames[0], 0, 0), [0, 0, 0, 0]);
        assert_eq!(canvas_pixel(&frames[1], 0, 0), [0, 0, 0, 0]);
        assert_eq!(canvas_pixel(&frames[2], 359, 556), [0, 0, 0, 0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_alpha_and_pixels_are_preserved_in_the_canvas() {
        let spec = SpriteSetSpec::SWEEP;
        let dir = temp_dir("alpha-preserved");
        for i in 0..SWEEP_FRAMES {
            let color = [10 + i as u8, 40, 90, 128];
            save_solid(&dir, spec, i, 5 + i as u32, 9 + i as u32, color);
        }
        let frames = spec.load_frames(&dir).expect("alpha frames must load");
        for (i, frame) in frames.iter().enumerate() {
            let (offset_x, offset_y) = frame.source_offset();
            let expected = [10 + i as u8, 40, 90, 128];
            assert_eq!(canvas_pixel(frame, offset_x, offset_y), expected);
            assert_eq!(canvas_pixel(frame, offset_x + 4, offset_y + 8), expected);
            if offset_x > 0 {
                assert_eq!(
                    canvas_pixel(frame, 0, offset_y),
                    [0, 0, 0, 0],
                    "horizontal padding must remain fully transparent"
                );
            }
            if offset_y > 0 {
                assert_eq!(
                    canvas_pixel(frame, offset_x, 0),
                    [0, 0, 0, 0],
                    "vertical padding must remain fully transparent"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn canvas_uses_checked_arithmetic_and_a_documented_budget() {
        // Hostile dimensions must fail with a diagnostic, not overflow or
        // attempt to allocate.
        let err = checked_canvas_len(SpriteSetSpec::SWEEP, u32::MAX, u32::MAX).unwrap_err();
        assert!(matches!(err, SpriteError::CanvasTooLarge { .. }));
        let msg = err.to_string();
        assert!(msg.contains("sweep animation"));
        assert!(msg.contains("safety budget"));
        assert!(msg.contains("provide smaller PNGs"));

        let err = checked_canvas_len(SpriteSetSpec::HELP, 100_000, 100_000).unwrap_err();
        assert!(matches!(err, SpriteError::CanvasTooLarge { .. }));
    }

    #[test]
    fn missing_frame_diagnostic() {
        let dir = std::env::temp_dir().join("akari-no-such-sweep-dir");
        let err = SpriteSetSpec::SWEEP.load_frames(&dir).unwrap_err();
        assert!(err.to_string().contains("missing sprite frame 1"));
    }

    #[test]
    fn missing_idle_frame_diagnostic() {
        let dir = temp_dir("missing-idle");
        let err = SpriteSetSpec::IDLE.load_frames(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing sprite frame 1"));
        assert!(msg.contains("for idle set"));
        assert!(msg.contains("assets/akari/idle/idle_001..008.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_idle_frame_diagnostic() {
        let dir = temp_dir("corrupt-idle");
        std::fs::write(dir.join(SpriteSetSpec::IDLE.frame_name(0)), b"not a png").unwrap();
        let err = SpriteSetSpec::IDLE.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Load { .. }));
        assert!(err.to_string().contains("cannot decode"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_image_fails_gracefully() {
        let dir = temp_dir("unsupported-help");
        // A valid 1x1 GIF stored under a .png name: the loader expects PNG
        // and must report an actionable decode failure.
        let gif: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\
            \xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\
            \x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
        std::fs::write(dir.join(SpriteSetSpec::HELP.frame_name(0)), gif).unwrap();
        let err = SpriteSetSpec::HELP.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Load { .. }));
        assert!(err.to_string().contains("cannot decode"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_health_frame_diagnostic() {
        let dir = temp_dir("missing-health");
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing sprite frame 1"));
        assert!(msg.contains("for health set"));
        assert!(msg.contains("assets/akari/health/health_001..008.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_health_set_reports_first_missing_frame() {
        let dir = temp_dir("incomplete-health");
        save_solid(&dir, SpriteSetSpec::HEALTH, 0, 8, 8, [1, 2, 3, 255]);
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Missing { index: 1, .. }));
        assert!(err.to_string().contains("missing sprite frame 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_health_frame_diagnostic() {
        let dir = temp_dir("corrupt-health");
        std::fs::write(
            dir.join(SpriteSetSpec::HEALTH.frame_name(0)),
            b"\x89PNG\r\n\x1a\ntruncated",
        )
        .unwrap();
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Load { .. }));
        assert!(err.to_string().contains("cannot decode"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_about_frame_diagnostic() {
        let dir = temp_dir("missing-about");
        let err = SpriteSetSpec::ABOUT.load_frames(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing sprite frame 1"));
        assert!(msg.contains("for about set"));
        assert!(msg.contains("assets/akari/about/about_001..004.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_about_frame_diagnostic() {
        let dir = temp_dir("corrupt-about");
        std::fs::write(dir.join(SpriteSetSpec::ABOUT.frame_name(0)), b"not a png").unwrap();
        let err = SpriteSetSpec::ABOUT.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Load { .. }));
        assert!(err.to_string().contains("cannot decode"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_help_frame_diagnostic() {
        let dir = temp_dir("missing-help");
        let err = SpriteSetSpec::HELP.load_frames(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing sprite frame 1"));
        assert!(msg.contains("for help set"));
        assert!(msg.contains("assets/akari/help/help_001..004.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_help_frame_diagnostic() {
        let dir = temp_dir("corrupt-help");
        std::fs::write(dir.join(SpriteSetSpec::HELP.frame_name(0)), b"not a png").unwrap();
        let err = SpriteSetSpec::HELP.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Load { .. }));
        assert!(err.to_string().contains("cannot decode"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
