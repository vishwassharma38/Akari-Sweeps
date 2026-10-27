//! Sprite discovery, validation, and pixel loading.
//!
//! Each animation set is a sequence of RGBA PNGs in its own
//! `assets/akari/<set>/` directory, validated at the set's documented
//! size. They are never rewritten; the TUI renders a scaled half-block
//! approximation (documented limitation) with transparency blended.

use std::path::{Path, PathBuf};

pub const SWEEP_WIDTH: u32 = 256;
pub const SWEEP_HEIGHT: u32 = 320;
pub const SWEEP_FRAMES: usize = 8;
pub const IDLE_WIDTH: u32 = 367;
pub const IDLE_HEIGHT: u32 = 564;
pub const IDLE_FRAMES: usize = 8;
pub const HEALTH_WIDTH: u32 = 358;
pub const HEALTH_HEIGHT: u32 = 575;
pub const HEALTH_FRAMES: usize = 8;
/// Target 6 frames per second (≈166.7 ms per frame). All animation sets
/// share this timing policy through `Animator`.
pub const SWEEP_INTERVAL_MS: u64 = 167;

/// Filesystem identity and validation constraints for one animation set.
/// Future sets (about/help) add a constant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSetSpec {
    /// Asset subdirectory under `assets/akari/`.
    pub dir: &'static str,
    /// File-name prefix before `_NNN.png`.
    pub prefix: &'static str,
    pub frame_count: usize,
    pub width: u32,
    pub height: u32,
}

impl SpriteSetSpec {
    pub const SWEEP: Self = Self {
        dir: "sweep",
        prefix: "akari_sweep",
        frame_count: SWEEP_FRAMES,
        width: SWEEP_WIDTH,
        height: SWEEP_HEIGHT,
    };

    pub const IDLE: Self = Self {
        dir: "idle",
        prefix: "idle",
        frame_count: IDLE_FRAMES,
        width: IDLE_WIDTH,
        height: IDLE_HEIGHT,
    };

    pub const HEALTH: Self = Self {
        dir: "health",
        prefix: "health",
        frame_count: HEALTH_FRAMES,
        width: HEALTH_WIDTH,
        height: HEALTH_HEIGHT,
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

    /// Load and validate every frame in order.
    pub fn load_frames(self, dir: &Path) -> Result<Vec<SpriteFrame>, SpriteError> {
        let mut frames = Vec::with_capacity(self.frame_count);
        for i in 0..self.frame_count {
            let path = dir.join(self.frame_name(i));
            if !path.exists() {
                return Err(SpriteError::Missing {
                    spec: self,
                    index: i,
                    path,
                });
            }
            let img = image::open(&path).map_err(|e| SpriteError::Load {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            if w != self.width || h != self.height {
                return Err(SpriteError::BadDimensions {
                    spec: self,
                    path,
                    w,
                    h,
                });
            }
            frames.push(SpriteFrame {
                width: w,
                height: h,
                rgba: rgba.into_raw(),
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
    BadDimensions {
        spec: SpriteSetSpec,
        path: PathBuf,
        w: u32,
        h: u32,
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
            SpriteError::BadDimensions { spec, path, w, h } => write!(
                f,
                "{} is {w}x{h}, expected {}x{} for the {} animation — re-export at the documented size",
                path.display(),
                spec.width,
                spec.height,
                spec.dir
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

/// Load and validate all eight sweep frames in order.
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
    }

    #[test]
    fn real_assets_load_at_documented_size() {
        for spec in [
            SpriteSetSpec::SWEEP,
            SpriteSetSpec::IDLE,
            SpriteSetSpec::HEALTH,
        ] {
            let dir = manifest_dir(spec);
            if !dir.join(spec.frame_name(0)).exists() {
                eprintln!("skipping: {} assets not in tree", spec.dir);
                continue;
            }
            let frames = spec.load_frames(&dir).expect("frames must decode");
            assert_eq!(frames.len(), spec.frame_count);
            for f in &frames {
                assert_eq!((f.width, f.height), (spec.width, spec.height));
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
            let expected = image::open(dir.join(spec.frame_name(i)))
                .expect("source frame must decode")
                .to_rgba8();
            assert_eq!(
                frame.rgba,
                expected.into_raw(),
                "idle frame {i} loaded out of order"
            );
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
            let expected = image::open(dir.join(spec.frame_name(i)))
                .expect("source frame must decode")
                .to_rgba8();
            assert_eq!(
                frame.rgba,
                expected.into_raw(),
                "health frame {i} loaded out of order"
            );
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
        let img = image::RgbaImage::new(HEALTH_WIDTH, HEALTH_HEIGHT);
        img.save(dir.join(SpriteSetSpec::HEALTH.frame_name(0)))
            .unwrap();
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Missing { index: 1, .. }));
        assert!(err.to_string().contains("missing sprite frame 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_health_frame_diagnostic() {
        let dir = temp_dir("corrupt-health");
        std::fs::write(dir.join(SpriteSetSpec::HEALTH.frame_name(0)), b"not a png").unwrap();
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::Load { .. }));
        assert!(err.to_string().contains("cannot decode"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_health_dimensions_report_health_size() {
        let dir = temp_dir("bad-health-dim");
        let img = image::RgbaImage::new(4, 4);
        img.save(dir.join(SpriteSetSpec::HEALTH.frame_name(0)))
            .unwrap();
        let err = SpriteSetSpec::HEALTH.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::BadDimensions { .. }));
        let msg = err.to_string();
        assert!(msg.contains("expected 358x575"));
        assert!(msg.contains("health animation"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_dimensions_rejected() {
        let dir = temp_dir("bad-dim");
        let img = image::RgbaImage::new(4, 4);
        img.save(dir.join(SpriteSetSpec::SWEEP.frame_name(0)))
            .unwrap();
        let err = SpriteSetSpec::SWEEP.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::BadDimensions { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_idle_dimensions_report_idle_size() {
        let dir = temp_dir("bad-idle-dim");
        let img = image::RgbaImage::new(4, 4);
        img.save(dir.join(SpriteSetSpec::IDLE.frame_name(0)))
            .unwrap();
        let err = SpriteSetSpec::IDLE.load_frames(&dir).unwrap_err();
        assert!(matches!(err, SpriteError::BadDimensions { .. }));
        let msg = err.to_string();
        assert!(msg.contains("expected 367x564"));
        assert!(msg.contains("idle animation"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
