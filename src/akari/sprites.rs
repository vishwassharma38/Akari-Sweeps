//! Sprite discovery, normalization, and pixel loading.
//!
//! Each animation set is a sequence of PNGs in its own
//! `assets/akari/<set>/` directory. Frames are discovered from the
//! filesystem: every `<prefix>_<number>.png` file is loaded, sorted by its
//! numeric index. There is no hardcoded frame limit and no content-based
//! deduplication, so byte-identical files remain separate animation frames.
//!
//! Replacement frames may use any dimensions: the loader decodes every
//! discovered frame, derives one shared canvas from the largest width and
//! height in the set, and centers each source image on a transparent canvas
//! of that size. Frames are never rewritten; the TUI renders a scaled
//! half-block approximation (documented limitation) with transparent pixels
//! blended over the terminal background.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Target 6 frames per second (≈166.7 ms per frame). All animation sets
/// share this timing policy through `Animator`.
pub const SWEEP_INTERVAL_MS: u64 = 167;

/// Safety budget for the normalized RGBA data of one animation set (all
/// frames). This is deliberately far above any sensible terminal sprite and
/// is a resource guard, not an expected image size: replacement art may use
/// any dimensions or aspect ratio within it.
pub const MAX_ANIMATION_BYTES: usize = 256 * 1024 * 1024;

/// Filesystem identity and loading constraints for one animation set.
/// Additional sets add a constant here. Frame counts are discovered from
/// the directory, so they are deliberately not part of this spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSetSpec {
    /// Asset directory under `assets/akari/` (may be nested).
    pub dir: &'static str,
    /// File-name prefix before `_NNN.png`.
    pub prefix: &'static str,
}

impl SpriteSetSpec {
    pub const SWEEP: Self = Self {
        dir: "sweep",
        prefix: "sweep",
    };

    pub const IDLE: Self = Self {
        dir: "idle",
        prefix: "idle",
    };

    pub const HEALTH: Self = Self {
        dir: "health",
        prefix: "health",
    };

    pub const ABOUT: Self = Self {
        dir: "about",
        prefix: "about",
    };

    pub const HELP: Self = Self {
        dir: "help",
        prefix: "help",
    };

    /// Canonical file name for frame `index` (0-based).
    pub fn frame_name(self, index: usize) -> String {
        format!("{}_{:03}.png", self.prefix, index + 1)
    }

    /// Diagnostic hint naming the supported asset naming convention.
    pub fn asset_hint(self) -> String {
        format!("assets/akari/{}/{}_NNN.png", self.dir, self.prefix)
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

    /// Resolve this asset directory: the first candidate that contains at
    /// least one numbered PNG frame. A directory with gaps or malformed PNG
    /// names still resolves so `load_frames` can report the precise problem.
    pub fn resolve_dir(self) -> Option<PathBuf> {
        self.candidate_dirs()
            .into_iter()
            .find(|dir| self.contains_frame_candidates(dir))
    }

    fn contains_frame_candidates(self, dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| self.parse_frame_index(name).is_some())
                })
            })
            .unwrap_or(false)
    }

    /// Parse `prefix_NNN.png` into a 0-based frame index. The number must be
    /// positive; any number of digits is accepted, so `_009` and `_010`
    /// continue the sequence naturally.
    fn parse_frame_index(self, file_name: &str) -> Option<usize> {
        let (stem, extension) = file_name.rsplit_once('.')?;
        if !extension.eq_ignore_ascii_case("png") {
            return None;
        }
        let digits = stem.strip_prefix(self.prefix)?.strip_prefix('_')?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let number: usize = digits.parse().ok()?;
        (number > 0).then_some(number - 1)
    }

    /// Enumerate every numbered frame file in ascending numeric order.
    ///
    /// Non-PNG files (README documents, checksums, etc.) are ignored. PNG
    /// files that do not follow the registered naming convention are
    /// reported explicitly instead of being silently loaded or skipped.
    /// Numeric gaps and duplicate indices are rejected with actionable
    /// diagnostics so frame order can never become ambiguous.
    fn discover_frames(self, dir: &Path) -> Result<Vec<PathBuf>, SpriteError> {
        if !dir.is_dir() {
            return Err(SpriteError::Missing {
                spec: self,
                index: 0,
                path: dir.join(self.frame_name(0)),
            });
        }

        let entries = std::fs::read_dir(dir).map_err(|error| SpriteError::ReadDir {
            path: dir.to_path_buf(),
            reason: error.to_string(),
        })?;
        let mut by_index: BTreeMap<usize, PathBuf> = BTreeMap::new();
        let mut unexpected = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|error| SpriteError::ReadDir {
                path: dir.to_path_buf(),
                reason: error.to_string(),
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| SpriteError::ReadDir {
                path: path.clone(),
                reason: error.to_string(),
            })?;
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some(index) = self.parse_frame_index(&name) {
                if let Some(previous) = by_index.insert(index, path.clone()) {
                    return Err(SpriteError::DuplicateIndex {
                        spec: self,
                        index: index + 1,
                        first: previous,
                        second: path,
                    });
                }
            } else if name.to_ascii_lowercase().ends_with(".png") {
                unexpected.push(path);
            }
        }

        if !unexpected.is_empty() {
            unexpected.sort();
            return Err(SpriteError::UnexpectedPng {
                spec: self,
                paths: unexpected,
            });
        }

        if by_index.is_empty() {
            return Err(SpriteError::Missing {
                spec: self,
                index: 0,
                path: dir.join(self.frame_name(0)),
            });
        }

        // Frame numbering must be contiguous and 1-based. A gap would leave
        // an animation position undefined, so fail with the missing numbers.
        let max_index = *by_index.keys().next_back().expect("non-empty discovery");
        let missing: Vec<usize> = (0..=max_index)
            .filter(|index| !by_index.contains_key(index))
            .map(|index| index + 1)
            .collect();
        if !missing.is_empty() {
            return Err(SpriteError::MissingSequence {
                spec: self,
                found: by_index.keys().map(|index| index + 1).collect(),
                missing,
            });
        }

        Ok(by_index.into_values().collect())
    }

    /// Public path-only discovery used by diagnostics and tests.
    #[allow(dead_code)]
    pub fn discover_frame_paths(self, dir: &Path) -> Result<Vec<PathBuf>, SpriteError> {
        self.discover_frames(dir)
    }

    /// Load every discovered frame in numeric order and normalize the set
    /// onto one shared canvas sized to the largest source width/height.
    /// Every source image is centered, unmodified, on transparent padding.
    /// Byte-identical files remain separate frame entries.
    pub fn load_frames(self, dir: &Path) -> Result<Vec<SpriteFrame>, SpriteError> {
        let discovered = self.discover_frames(dir)?;
        let frame_count = discovered.len();

        // Pass 1: resolve paths and read header dimensions without keeping
        // full pixel buffers. This lets the shared canvas be sized before
        // any unbounded RGBA allocation happens.
        let mut sources = Vec::with_capacity(frame_count);
        for path in discovered {
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
        let canvas_len = checked_canvas_len(self, canvas_width, canvas_height, frame_count)?;

        // Pass 2: decode one frame at a time and copy it into its centered
        // canvas position. Pixels (including alpha) are copied verbatim.
        let mut frames = Vec::with_capacity(frame_count);
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
                    frames: frame_count,
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
/// `frame_count` is the number of frames actually discovered on disk, so the
/// memory budget always reflects the real animation rather than a hardcoded
/// count. Checked arithmetic prevents overflow for hostile dimensions; the
/// budget keeps memory use bounded without reintroducing a fixed asset size.
fn checked_canvas_len(
    spec: SpriteSetSpec,
    width: u32,
    height: u32,
    frame_count: usize,
) -> Result<usize, SpriteError> {
    let total_bytes = (width as u128)
        .checked_mul(height as u128)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| bytes.checked_mul(frame_count as u128))
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
            frames: frame_count,
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
    MissingSequence {
        spec: SpriteSetSpec,
        found: Vec<usize>,
        missing: Vec<usize>,
    },
    DuplicateIndex {
        spec: SpriteSetSpec,
        index: usize,
        first: PathBuf,
        second: PathBuf,
    },
    UnexpectedPng {
        spec: SpriteSetSpec,
        paths: Vec<PathBuf>,
    },
    ReadDir {
        path: PathBuf,
        reason: String,
    },
    Load {
        path: PathBuf,
        reason: String,
    },
    CanvasTooLarge {
        spec: SpriteSetSpec,
        width: u32,
        height: u32,
        frames: usize,
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
            SpriteError::MissingSequence {
                spec,
                found,
                missing,
            } => write!(
                f,
                "missing sprite frame{} {} for {} set: numbering must be contiguous from 001 \
                 (found {}); restore {}",
                if missing.len() == 1 { "" } else { "s" },
                join_numbers(missing),
                spec.dir,
                join_numbers(found),
                spec.asset_hint()
            ),
            SpriteError::DuplicateIndex {
                spec,
                index,
                first,
                second,
            } => write!(
                f,
                "duplicate frame number {index:03} for {} set: {} and {} both map to that number — \
                 keep one file per frame in {}",
                spec.dir,
                first.display(),
                second.display(),
                spec.asset_hint()
            ),
            SpriteError::UnexpectedPng { spec, paths } => write!(
                f,
                "unexpected PNG file{} in the {} set: {} — rename to match {} or remove \
                 (only numbered frames are loaded)",
                if paths.len() == 1 { "" } else { "s" },
                spec.dir,
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                spec.asset_hint()
            ),
            SpriteError::ReadDir { path, reason } => {
                write!(
                    f,
                    "cannot read sprite directory {}: {reason}",
                    path.display()
                )
            }
            SpriteError::Load { path, reason } => {
                write!(f, "cannot decode {}: {reason}", path.display())
            }
            SpriteError::CanvasTooLarge {
                spec,
                width,
                height,
                frames,
                limit,
            } => write!(
                f,
                "cannot normalize the {} animation: shared {width}x{height} canvas for {frames} \
                 frames exceeds the {} MiB safety budget — provide smaller PNGs",
                spec.dir,
                limit / (1024 * 1024)
            ),
        }
    }
}

/// Format frame numbers as `001, 004` for diagnostics.
fn join_numbers(numbers: &[usize]) -> String {
    numbers
        .iter()
        .map(|number| format!("{number:03}"))
        .collect::<Vec<_>>()
        .join(", ")
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

/// Load and normalize every discovered sweep frame in numeric order.
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

    fn source_image(path: &Path) -> image::RgbaImage {
        image::open(path)
            .expect("source frame must decode")
            .to_rgba8()
    }

    /// Independently enumerate numbered PNGs on disk for shipped-art checks.
    /// This intentionally does not call the loader's discovery function.
    fn numbered_png_paths(dir: &Path, spec: SpriteSetSpec) -> Vec<PathBuf> {
        let filename_prefix = format!("{}_", spec.prefix);
        let mut numbered = std::fs::read_dir(dir)
            .expect("shipped sprite directory must be readable")
            .map(|entry| entry.expect("directory entry must be readable").path())
            .filter_map(|path| {
                let extension = path.extension()?.to_str()?;
                if !extension.eq_ignore_ascii_case("png") {
                    return None;
                }
                let stem = path.file_stem()?.to_str()?;
                let number = stem.strip_prefix(&filename_prefix)?;
                if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                let number = number.parse::<usize>().ok()?;
                (number > 0).then_some((number, path))
            })
            .collect::<Vec<_>>();
        numbered.sort_by_key(|(number, _)| *number);
        numbered.into_iter().map(|(_, path)| path).collect()
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
    fn frame_names_follow_the_numbering_convention_for_all_sets() {
        for spec in [
            SpriteSetSpec::SWEEP,
            SpriteSetSpec::IDLE,
            SpriteSetSpec::HEALTH,
            SpriteSetSpec::ABOUT,
            SpriteSetSpec::HELP,
        ] {
            assert_eq!(spec.frame_name(0), format!("{}_001.png", spec.prefix));
            assert_eq!(spec.frame_name(8), format!("{}_009.png", spec.prefix));
            assert_eq!(spec.frame_name(9), format!("{}_010.png", spec.prefix));
        }
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
            let expected_paths = numbered_png_paths(&dir, spec);
            assert!(
                !expected_paths.is_empty(),
                "{} must have numbered PNGs",
                spec.dir
            );
            let discovered = spec
                .discover_frame_paths(&resolved)
                .expect("all shipped numbered PNGs must be discovered");
            assert_eq!(discovered, expected_paths, "{} discovery order", spec.dir);
            let frames = spec.load_frames(&resolved).expect("frames must decode");
            assert_eq!(
                frames.len(),
                expected_paths.len(),
                "{} frame count",
                spec.dir
            );

            let max_width = frames.iter().map(|f| f.source_width).max().unwrap();
            let max_height = frames.iter().map(|f| f.source_height).max().unwrap();
            for (i, (frame, path)) in frames.iter().zip(&expected_paths).enumerate() {
                assert_eq!(
                    (frame.canvas_width, frame.canvas_height),
                    (max_width, max_height),
                    "{} frame {i} must share the set canvas",
                    spec.dir
                );
                assert_source_preserved(frame, &source_image(path));
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
            let expected_paths = numbered_png_paths(&dir, spec);
            let frames = spec.load_frames(&dir).expect("frames must decode");
            assert_eq!(frames.len(), expected_paths.len());
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
    fn shipped_asset_tree_loads_every_numbered_png() {
        let specs = [
            SpriteSetSpec::SWEEP,
            SpriteSetSpec::IDLE,
            SpriteSetSpec::HEALTH,
            SpriteSetSpec::ABOUT,
            SpriteSetSpec::HELP,
        ];
        for spec in specs {
            let dir = manifest_dir(spec);
            let expected_paths = numbered_png_paths(&dir, spec);
            let discovered = spec
                .discover_frame_paths(&dir)
                .unwrap_or_else(|error| panic!("{} assets must discover: {error}", spec.dir));
            assert_eq!(discovered, expected_paths, "{} discovered paths", spec.dir);
            let frames = spec
                .load_frames(&dir)
                .unwrap_or_else(|error| panic!("{} assets must load: {error}", spec.dir));
            assert_eq!(
                frames.len(),
                expected_paths.len(),
                "{} loaded count",
                spec.dir
            );
        }
    }

    #[test]
    fn byte_identical_shipped_frames_remain_separate_entries() {
        for spec in [SpriteSetSpec::ABOUT, SpriteSetSpec::HELP] {
            let dir = manifest_dir(spec);
            let expected_paths = numbered_png_paths(&dir, spec);
            let frames = spec.load_frames(&dir).expect("duplicate frames must load");
            let source_bytes = expected_paths
                .iter()
                .map(|path| std::fs::read(path).expect("shipped PNG must be readable"))
                .collect::<Vec<_>>();
            let mut duplicate_pairs = Vec::new();
            for first in 0..source_bytes.len() {
                for duplicate in first + 1..source_bytes.len() {
                    if source_bytes[first] == source_bytes[duplicate] {
                        duplicate_pairs.push((first, duplicate));
                    }
                }
            }
            assert!(
                !duplicate_pairs.is_empty(),
                "{} shipped artwork should exercise duplicate preservation",
                spec.dir
            );
            for (first, duplicate) in duplicate_pairs {
                assert_eq!(
                    frames[first].rgba, frames[duplicate].rgba,
                    "{} byte-identical source files must decode identically",
                    spec.dir
                );
                assert_ne!(
                    frames[first].rgba.as_ptr(),
                    frames[duplicate].rgba.as_ptr(),
                    "{} duplicate files must remain separate loaded entries",
                    spec.dir
                );
            }
            for (index, frame) in frames.iter().enumerate() {
                assert_source_preserved(frame, &source_image(&expected_paths[index]));
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
        for (frame, path) in frames.iter().zip(numbered_png_paths(&dir, spec)) {
            assert_source_preserved(frame, &source_image(&path));
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
        for (frame, path) in frames.iter().zip(numbered_png_paths(&dir, spec)) {
            assert_source_preserved(frame, &source_image(&path));
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
        for (frame, path) in frames.iter().zip(numbered_png_paths(&dir, spec)) {
            assert_source_preserved(frame, &source_image(&path));
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
        for (frame, path) in frames.iter().zip(numbered_png_paths(&dir, spec)) {
            assert_source_preserved(frame, &source_image(&path));
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
        for i in 0..4 {
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
        let err = checked_canvas_len(SpriteSetSpec::SWEEP, u32::MAX, u32::MAX, 4).unwrap_err();
        assert!(matches!(err, SpriteError::CanvasTooLarge { .. }));
        let msg = err.to_string();
        assert!(msg.contains("sweep animation"));
        assert!(msg.contains("4 frames"));
        assert!(msg.contains("safety budget"));
        assert!(msg.contains("provide smaller PNGs"));

        let err = checked_canvas_len(SpriteSetSpec::HELP, 100_000, 100_000, 4).unwrap_err();
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
        assert!(msg.contains("assets/akari/idle/idle_NNN.png"));
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
        assert!(msg.contains("assets/akari/health/health_NNN.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_health_set_reports_missing_sequence_number() {
        let dir = temp_dir("incomplete-health");
        save_solid(&dir, SpriteSetSpec::HEALTH, 0, 8, 8, [1, 2, 3, 255]);
        save_solid(&dir, SpriteSetSpec::HEALTH, 1, 8, 8, [1, 2, 3, 255]);
        // Frame 003 is missing, so positions 001, 002, 004 are ambiguous.
        save_solid(&dir, SpriteSetSpec::HEALTH, 3, 8, 8, [1, 2, 3, 255]);
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::MissingSequence { .. }));
        let msg = err.to_string();
        assert!(msg.contains("missing sprite frame 003"));
        assert!(msg.contains("for health set"));
        assert!(msg.contains("found 001, 002, 004"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn byte_identical_frames_are_loaded_as_separate_entries() {
        let spec = SpriteSetSpec::ABOUT;
        let dir = temp_dir("duplicate-frames");
        save_solid(&dir, spec, 0, 6, 5, [12, 34, 56, 255]);
        let bytes = std::fs::read(dir.join(spec.frame_name(0))).unwrap();
        for index in 1..8 {
            std::fs::write(dir.join(spec.frame_name(index)), &bytes).unwrap();
        }

        let frames = spec.load_frames(&dir).expect("duplicate PNGs must load");
        assert_eq!(frames.len(), 8, "every filename must produce a frame");
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                (frame.source_width, frame.source_height),
                (6, 5),
                "frame {index} must retain its source dimensions"
            );
            assert_eq!(
                frame.rgba, frames[0].rgba,
                "frame {index} must decode its own identical pixels"
            );
            assert_source_preserved(frame, &source_image(&dir.join(spec.frame_name(index))));
        }
        assert_ne!(
            frames[0].rgba.as_ptr(),
            frames[4].rgba.as_ptr(),
            "duplicate frames must be independent allocations, never deduplicated"
        );

        let paths = spec.discover_frame_paths(&dir).expect("frames discover");
        let names: Vec<_> = paths
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            (0..8)
                .map(|index| spec.frame_name(index))
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_loads_every_numbered_frame_in_numeric_order() {
        let spec = SpriteSetSpec::HELP;
        let dir = temp_dir("ten-frames");
        for number in 1..=10u8 {
            let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([number, 10, 20, 255]));
            img.save(dir.join(format!("help_{number:03}.png"))).unwrap();
        }

        let frames = spec.load_frames(&dir).expect("ten frames must load");
        assert_eq!(frames.len(), 10, "one fixture PNG must load per frame");
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                canvas_pixel(frame, 0, 0),
                [index as u8 + 1, 10, 20, 255],
                "frame {index} must load in ascending numeric filename order"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_orders_unpadded_numbers_numerically_not_lexicographically() {
        let spec = SpriteSetSpec::HELP;
        let dir = temp_dir("numeric-sort");
        for number in 1..=10u8 {
            let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([number, 0, 0, 255]));
            img.save(dir.join(format!("help_{number}.png"))).unwrap();
        }

        let frames = spec.load_frames(&dir).expect("unpadded frames must load");
        assert_eq!(frames.len(), 10);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                canvas_pixel(frame, 0, 0),
                [index as u8 + 1, 0, 0, 255],
                "frame {index} must follow numeric, not lexical, order"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unexpected_png_files_are_reported_not_loaded() {
        let spec = SpriteSetSpec::ABOUT;
        let dir = temp_dir("unexpected-png");
        save_solid(&dir, spec, 0, 4, 4, [1, 2, 3, 255]);
        let img = image::RgbaImage::from_pixel(3, 3, image::Rgba([9, 9, 9, 255]));
        img.save(dir.join("poster.png")).unwrap();
        img.save(dir.join("about_bonus.png")).unwrap();

        let err = spec.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::UnexpectedPng { .. }));
        let msg = err.to_string();
        assert!(msg.contains("unexpected PNG"));
        assert!(msg.contains("poster.png"));
        assert!(msg.contains("about_bonus.png"));
        assert!(msg.contains("only numbered frames are loaded"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn readme_and_non_png_files_are_ignored() {
        let spec = SpriteSetSpec::ABOUT;
        let dir = temp_dir("readme-ignored");
        save_solid(&dir, spec, 0, 4, 4, [1, 2, 3, 255]);
        std::fs::write(dir.join("README.md"), b"replacement instructions").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a frame").unwrap();

        let frames = spec
            .load_frames(&dir)
            .expect("README must not block loading");
        assert_eq!(frames.len(), 1);
        assert_eq!((frames[0].source_width, frames[0].source_height), (4, 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_numeric_indices_are_reported() {
        let spec = SpriteSetSpec::ABOUT;
        let dir = temp_dir("duplicate-index");
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
        img.save(dir.join("about_1.png")).unwrap();
        img.save(dir.join("about_001.png")).unwrap();

        let err = spec.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::DuplicateIndex { .. }));
        let msg = err.to_string();
        assert!(msg.contains("duplicate frame number 001"));
        assert!(msg.contains("about_1.png") && msg.contains("about_001.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directories_with_a_broken_sequence_are_still_resolved() {
        let dir = temp_dir("broken-sequence-resolve");
        save_solid(&dir, SpriteSetSpec::HELP, 0, 4, 4, [1, 2, 3, 255]);
        save_solid(&dir, SpriteSetSpec::HELP, 2, 4, 4, [1, 2, 3, 255]);
        assert!(
            SpriteSetSpec::HELP.discover_frame_paths(&dir).is_err(),
            "the gap must be diagnosed"
        );
        // The directory is still a legitimate candidate so the TUI can
        // surface the precise diagnostic instead of "assets not found".
        assert!(SpriteSetSpec::HELP.contains_frame_candidates(&dir));
        assert!(matches!(
            SpriteSetSpec::HELP.load_frames(&dir),
            Err(SpriteError::MissingSequence { .. })
        ));
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
        assert!(msg.contains("assets/akari/about/about_NNN.png"));
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
        assert!(msg.contains("assets/akari/help/help_NNN.png"));
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
