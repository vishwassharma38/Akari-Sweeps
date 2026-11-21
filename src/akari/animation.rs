//! Frame sequencing and timing. Animation sets (sweep/idle/health/about/help)
//! plug in here without touching the TUI.

use crate::akari::sprites::{SWEEP_INTERVAL_MS, SpriteSetSpec};

/// Which animation set is displayed. Sweep runs during cleanup; Health is
/// the nurse-inspired loop shown during health/repair work; Idle is the
/// book-reading loop shown while Home waits for input; About is the
/// magnifying-glass loop shown on the About screen; Help is its own loop
/// shown on the Help screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationSet {
    Sweep,
    Idle,
    Health,
    About,
    Help,
}

impl AnimationSet {
    pub const COUNT: usize = 5;
    /// Every set with shipped frames, in load order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Sweep,
        Self::Idle,
        Self::Health,
        Self::About,
        Self::Help,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Sweep => 0,
            Self::Idle => 1,
            Self::Health => 2,
            Self::About => 3,
            Self::Help => 4,
        }
    }

    /// Asset metadata used by the shared discovery/loading pipeline.
    pub fn sprite_set(self) -> SpriteSetSpec {
        match self {
            Self::Sweep => SpriteSetSpec::SWEEP,
            Self::Idle => SpriteSetSpec::IDLE,
            Self::Health => SpriteSetSpec::HEALTH,
            Self::About => SpriteSetSpec::ABOUT,
            Self::Help => SpriteSetSpec::HELP,
        }
    }
}

/// Playback state driven by `App::execution_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationMode {
    /// Home/About/Help: gentle loop so the panel is alive but calm.
    Ambient,
    /// Maintenance actively running: full-speed loop for the selected set.
    Sweeping,
    /// Work finished: hold the final frame (not "still sweeping").
    Settled,
}

#[derive(Debug)]
pub struct Animator {
    set: AnimationSet,
    mode: AnimationMode,
    frame: usize,
    last_advance_ms: u64,
    frame_count: usize,
}

impl Animator {
    /// Build an animator for `set` using the number of frames actually
    /// loaded by `SpriteLibrary` (never a compile-time frame count).
    pub fn new(set: AnimationSet, frame_count: usize) -> Self {
        Self {
            set,
            mode: AnimationMode::Ambient,
            frame: 0,
            last_advance_ms: 0,
            frame_count,
        }
    }

    /// Select a new animation set without ever retaining an out-of-range
    /// frame index from the previous set. `frame_count` is the selected
    /// set's actual loaded frame count. The timing anchor is rebased to
    /// `now_ms` so a switch never fast-forwards the new loop.
    pub fn select_set(&mut self, set: AnimationSet, frame_count: usize, now_ms: u64) -> bool {
        if self.set == set {
            // The same set may have been reloaded/replaced (for example by
            // tests or a future runtime reload). Keep the current position
            // when still valid but always adopt the real count.
            self.sync_frame_count(frame_count);
            return false;
        }
        self.set = set;
        self.frame_count = frame_count;
        self.frame = 0;
        self.last_advance_ms = now_ms;
        true
    }

    /// Adopt a new loaded frame count, clamping the current position so
    /// selection and Settled mode can never index past the loaded vector.
    pub fn sync_frame_count(&mut self, frame_count: usize) {
        self.frame_count = frame_count;
        self.frame = if frame_count == 0 {
            0
        } else {
            self.frame.min(frame_count - 1)
        };
    }

    /// Number of frames playback currently cycles through (the loaded
    /// vector length supplied by `SpriteLibrary`).
    #[allow(dead_code)]
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn set_mode(&mut self, mode: AnimationMode) {
        if self.mode != mode {
            self.mode = mode;
            if mode == AnimationMode::Settled {
                self.frame = self.frame_count.saturating_sub(1);
            }
        }
    }

    /// Begin a fresh run of the current set: first frame, timing anchored
    /// to `now_ms`. Used when a new operation starts so it cannot inherit
    /// the previous run's settled last frame.
    pub fn restart(&mut self, now_ms: u64) {
        self.frame = 0;
        self.last_advance_ms = now_ms;
    }

    #[cfg(test)]
    pub fn mode(&self) -> AnimationMode {
        self.mode
    }

    pub fn set(&self) -> AnimationSet {
        self.set
    }

    pub fn current_frame(&self) -> usize {
        self.frame
    }

    fn interval_ms(&self) -> u64 {
        match self.mode {
            AnimationMode::Ambient => SWEEP_INTERVAL_MS * 3,
            AnimationMode::Sweeping => SWEEP_INTERVAL_MS,
            AnimationMode::Settled => u64::MAX,
        }
    }

    /// Advance on monotonic wall-clock ms. Returns true when the frame
    /// changed. Deadline-based with resync: the clock is anchored to
    /// `now_ms` after advancing, so timing never accumulates drift, and
    /// a long stall advances by whole elapsed intervals at once instead
    /// of queuing a backlog (no spiral, no busy loop).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.mode == AnimationMode::Settled || self.frame_count == 0 {
            return false;
        }
        let interval = self.interval_ms();
        let elapsed = now_ms.wrapping_sub(self.last_advance_ms);
        if elapsed >= interval {
            let steps = elapsed / interval;
            self.last_advance_ms = now_ms;
            self.frame =
                (self.frame + (steps % self.frame_count as u64) as usize) % self.frame_count;
            return true;
        }
        false
    }

    /// Milliseconds until the next frame is due (saturating). The event
    /// loop polls for at most this long (capped), so frames land on time
    /// without blocking keyboard input.
    pub fn ms_until_next_frame(&self, now_ms: u64) -> u64 {
        if self.mode == AnimationMode::Settled || self.frame_count == 0 {
            return u64::MAX;
        }
        let interval = self.interval_ms();
        interval.saturating_sub(now_ms.wrapping_sub(self.last_advance_ms))
    }

    /// Pure helper for tests: frame index at tick `n`. A set with no loaded
    /// frames always resolves to index 0 without dividing by zero.
    #[allow(dead_code)]
    pub fn frame_at_tick(tick: u64, count: usize) -> usize {
        if count == 0 {
            0
        } else {
            (tick as usize) % count
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Pure Animator tests use an explicit synthetic fixture size. Shipped
    // asset counts are covered dynamically by the sprite and App tests.
    const TEST_FRAME_COUNT: usize = ["frame 0", "frame 1", "frame 2", "frame 3", "frame 4"].len();

    #[test]
    fn looping_visits_each_configured_frame_for_varied_counts() {
        let fixtures: [&[&str]; 3] = [
            &["only frame"],
            &["first", "middle", "last"],
            &["0", "1", "2", "3", "4"],
        ];
        for fixture in fixtures {
            let count = fixture.len();
            let mut a = Animator::new(AnimationSet::Sweep, count);
            a.set_mode(AnimationMode::Sweeping);
            let mut seen = std::collections::HashSet::new();
            for step in 1..=count as u64 {
                assert!(a.tick(step * SWEEP_INTERVAL_MS));
                seen.insert(a.current_frame());
            }
            assert_eq!(seen, (0..count).collect());
            assert_eq!(a.current_frame(), 0, "loop returns to its first frame");
        }
    }

    #[test]
    fn settled_holds_last_frame() {
        let mut a = Animator::new(AnimationSet::Sweep, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        a.tick(SWEEP_INTERVAL_MS);
        a.set_mode(AnimationMode::Settled);
        let held = a.current_frame();
        assert!(!a.tick(held as u64 + 10_000_000));
        assert_eq!(a.current_frame(), TEST_FRAME_COUNT - 1);
    }

    #[test]
    fn playback_targets_six_fps_without_drift() {
        // 6 FPS == ~166.7 ms per frame.
        assert_eq!(SWEEP_INTERVAL_MS, 167);
        let mut a = Animator::new(AnimationSet::Sweep, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        // Simulate 2 s of deadline ticks: expect 12 advances (±1 edge).
        let mut advances = 0;
        let mut t = 0u64;
        while t <= 2000 {
            if a.tick(t) {
                advances += 1;
            }
            t += 10;
        }
        assert!(
            (11..=13).contains(&advances),
            "expected ~12 frames in 2 s, got {advances}"
        );
    }

    #[test]
    fn stall_resyncs_without_backlog_spiral() {
        let mut a = Animator::new(AnimationSet::Sweep, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        a.tick(167);
        // A 10 s stall: one tick catches up (modulo frame count)…
        assert!(a.tick(10_167));
        // …and the very next tick has nothing to do (no backlog).
        assert!(!a.tick(10_167));
        assert!(!a.tick(10_200));
        // Scheduling resumes on deadline.
        assert_eq!(a.ms_until_next_frame(10_200), SWEEP_INTERVAL_MS - 33);
    }

    #[test]
    fn frame_at_tick_wraps() {
        assert_eq!(Animator::frame_at_tick(0, TEST_FRAME_COUNT), 0);
        assert_eq!(
            Animator::frame_at_tick(TEST_FRAME_COUNT as u64, TEST_FRAME_COUNT),
            0
        );
        assert_eq!(
            Animator::frame_at_tick(TEST_FRAME_COUNT as u64 + 1, TEST_FRAME_COUNT),
            1
        );
    }

    #[test]
    fn idle_set_uses_its_own_frame_count() {
        assert_eq!(AnimationSet::Idle.sprite_set(), SpriteSetSpec::IDLE);
        let a = Animator::new(AnimationSet::Idle, TEST_FRAME_COUNT);
        assert_eq!(a.set(), AnimationSet::Idle);
        assert_eq!(a.frame_count(), TEST_FRAME_COUNT);
    }

    #[test]
    fn idle_ambient_loops_through_all_frames() {
        let interval = SWEEP_INTERVAL_MS * 3;
        let mut a = Animator::new(AnimationSet::Idle, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Ambient);
        assert_eq!(a.ms_until_next_frame(0), interval);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(TEST_FRAME_COUNT * 2) {
            t += interval;
            assert!(a.tick(t), "ambient idle frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), TEST_FRAME_COUNT);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn health_set_uses_its_own_frame_count_and_frames() {
        assert_eq!(AnimationSet::Health.sprite_set(), SpriteSetSpec::HEALTH);
        let a = Animator::new(AnimationSet::Health, TEST_FRAME_COUNT);
        assert_eq!(a.set(), AnimationSet::Health);
        assert_eq!(a.frame_count(), TEST_FRAME_COUNT);
    }

    #[test]
    fn health_sweeping_loops_through_all_frames() {
        let mut a = Animator::new(AnimationSet::Health, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(TEST_FRAME_COUNT * 2) {
            t += SWEEP_INTERVAL_MS;
            assert!(a.tick(t), "health frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), TEST_FRAME_COUNT);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn about_set_uses_its_own_frame_count_and_frames() {
        assert_eq!(AnimationSet::About.sprite_set(), SpriteSetSpec::ABOUT);
        let a = Animator::new(AnimationSet::About, TEST_FRAME_COUNT);
        assert_eq!(a.set(), AnimationSet::About);
        assert_eq!(a.frame_count(), TEST_FRAME_COUNT);
    }

    #[test]
    fn about_ambient_loops_through_all_frames() {
        let interval = SWEEP_INTERVAL_MS * 3;
        let mut a = Animator::new(AnimationSet::About, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Ambient);
        assert_eq!(a.ms_until_next_frame(0), interval);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(TEST_FRAME_COUNT * 2) {
            t += interval;
            assert!(a.tick(t), "ambient about frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), TEST_FRAME_COUNT);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn help_set_uses_its_own_frame_count_and_frames() {
        assert_eq!(AnimationSet::Help.sprite_set(), SpriteSetSpec::HELP);
        let a = Animator::new(AnimationSet::Help, TEST_FRAME_COUNT);
        assert_eq!(a.set(), AnimationSet::Help);
        assert_eq!(a.frame_count(), TEST_FRAME_COUNT);
    }

    #[test]
    fn help_ambient_loops_through_all_frames() {
        let interval = SWEEP_INTERVAL_MS * 3;
        let mut a = Animator::new(AnimationSet::Help, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Ambient);
        assert_eq!(a.ms_until_next_frame(0), interval);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(TEST_FRAME_COUNT * 2) {
            t += interval;
            assert!(a.tick(t), "ambient help frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), TEST_FRAME_COUNT);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn animation_sets_are_indexed_in_load_order() {
        assert_eq!(AnimationSet::ALL.len(), AnimationSet::COUNT);
        for (i, set) in AnimationSet::ALL.iter().enumerate() {
            assert_eq!(set.index(), i, "{set:?} index must match load order");
        }
    }

    #[test]
    fn restart_rewinds_the_current_set_and_timing() {
        let mut a = Animator::new(AnimationSet::Sweep, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        a.tick(SWEEP_INTERVAL_MS * 3);
        assert!(a.current_frame() > 0);

        a.restart(10_000);
        assert_eq!(a.current_frame(), 0);
        assert_eq!(a.mode(), AnimationMode::Sweeping);
        assert_eq!(a.ms_until_next_frame(10_000), SWEEP_INTERVAL_MS);
        assert!(!a.tick(10_000 + SWEEP_INTERVAL_MS - 1));
        assert!(a.tick(10_000 + SWEEP_INTERVAL_MS));
        assert_eq!(a.current_frame(), 1);
    }

    #[test]
    fn switching_sets_resets_frame_and_stays_in_range() {
        let mut a = Animator::new(AnimationSet::Sweep, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        // Advance to the final sweep frame.
        a.tick(SWEEP_INTERVAL_MS * (TEST_FRAME_COUNT as u64 - 1));
        assert_eq!(a.current_frame(), TEST_FRAME_COUNT - 1);

        for (set, count, now) in [
            (AnimationSet::Idle, TEST_FRAME_COUNT, 10_000),
            (AnimationSet::Health, TEST_FRAME_COUNT, 20_000),
            (AnimationSet::About, TEST_FRAME_COUNT, 30_000),
            (AnimationSet::Help, TEST_FRAME_COUNT, 35_000),
        ] {
            assert!(a.select_set(set, count, now));
            assert_eq!(a.frame_count(), count);
            assert_eq!(a.current_frame(), 0);
            for step in 1..=(count as u64 * 2) {
                a.tick(now + step * SWEEP_INTERVAL_MS);
                assert!(a.current_frame() < count);
            }
        }

        assert!(a.select_set(AnimationSet::Sweep, TEST_FRAME_COUNT, 40_000));
        assert_eq!(a.frame_count(), TEST_FRAME_COUNT);
        assert_eq!(a.current_frame(), 0);
        assert!(!a.select_set(AnimationSet::Sweep, TEST_FRAME_COUNT, 41_000));
    }

    #[test]
    fn settled_mode_uses_the_actual_loaded_frame_count() {
        for (set, count) in [
            (AnimationSet::Sweep, 3),
            (AnimationSet::About, TEST_FRAME_COUNT),
            (AnimationSet::Help, TEST_FRAME_COUNT),
        ] {
            let mut a = Animator::new(set, count);
            a.set_mode(AnimationMode::Sweeping);
            a.tick(SWEEP_INTERVAL_MS);
            a.set_mode(AnimationMode::Settled);
            assert_eq!(
                a.current_frame(),
                count - 1,
                "{set:?} must hold its actual last loaded frame"
            );
            assert_eq!(a.ms_until_next_frame(u64::MAX / 2), u64::MAX);
        }
    }

    #[test]
    fn zero_loaded_frames_are_safe() {
        let mut a = Animator::new(AnimationSet::About, 0);
        assert_eq!(a.frame_count(), 0);
        assert_eq!(a.current_frame(), 0);
        assert_eq!(a.ms_until_next_frame(0), u64::MAX);

        a.set_mode(AnimationMode::Ambient);
        assert!(!a.tick(10_000_000), "no frames means no advancement");
        assert_eq!(a.current_frame(), 0);
        assert_eq!(Animator::frame_at_tick(7, 0), 0);

        a.set_mode(AnimationMode::Settled);
        assert_eq!(a.current_frame(), 0, "empty set settles at index 0 safely");
        assert!(!a.tick(20_000_000));
    }

    #[test]
    fn frame_count_updates_clamp_the_current_position() {
        let mut a = Animator::new(AnimationSet::Help, TEST_FRAME_COUNT);
        a.set_mode(AnimationMode::Sweeping);
        a.tick(SWEEP_INTERVAL_MS * (TEST_FRAME_COUNT as u64 - 1));
        assert_eq!(a.current_frame(), TEST_FRAME_COUNT - 1);

        // Same set, fewer loaded frames: adopt the new count and clamp.
        assert!(!a.select_set(AnimationSet::Help, 3, 10_000));
        assert_eq!(a.frame_count(), 3);
        assert_eq!(a.current_frame(), 2);
        for step in 1..=6 {
            a.tick(10_000 + step * SWEEP_INTERVAL_MS);
            assert!(a.current_frame() < 3);
        }

        // Same set, no frames: index 0 and no division by zero.
        a.sync_frame_count(0);
        assert_eq!(a.current_frame(), 0);
        assert!(!a.tick(100_000));
    }
}
