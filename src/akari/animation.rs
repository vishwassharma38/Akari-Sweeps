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

    pub fn frame_count(self) -> usize {
        self.sprite_set().frame_count
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
    pub fn new(set: AnimationSet) -> Self {
        Self {
            set,
            mode: AnimationMode::Ambient,
            frame: 0,
            last_advance_ms: 0,
            frame_count: set.frame_count(),
        }
    }

    /// Select a new animation set without ever retaining an out-of-range
    /// frame index from the previous set. The timing anchor is rebased to
    /// `now_ms` so a switch never fast-forwards the new loop.
    pub fn select_set(&mut self, set: AnimationSet, now_ms: u64) -> bool {
        if self.set == set {
            return false;
        }
        self.set = set;
        self.frame_count = set.frame_count();
        self.frame = 0;
        self.last_advance_ms = now_ms;
        true
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
        if self.mode == AnimationMode::Settled {
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
        if self.mode == AnimationMode::Settled {
            return u64::MAX;
        }
        let interval = self.interval_ms();
        interval.saturating_sub(now_ms.wrapping_sub(self.last_advance_ms))
    }

    /// Pure helper for tests: frame index at tick `n`.
    #[allow(dead_code)]
    pub fn frame_at_tick(tick: u64, count: usize) -> usize {
        (tick as usize) % count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::akari::sprites::{
        ABOUT_FRAMES, HEALTH_FRAMES, HELP_FRAMES, IDLE_FRAMES, SWEEP_FRAMES,
    };

    #[test]
    fn loops_through_eight_frames() {
        let mut a = Animator::new(AnimationSet::Sweep);
        a.set_mode(AnimationMode::Sweeping);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..16 {
            t += SWEEP_INTERVAL_MS;
            a.tick(t);
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn settled_holds_last_frame() {
        let mut a = Animator::new(AnimationSet::Sweep);
        a.set_mode(AnimationMode::Sweeping);
        a.tick(SWEEP_INTERVAL_MS);
        a.set_mode(AnimationMode::Settled);
        let held = a.current_frame();
        assert!(!a.tick(held as u64 + 10_000_000));
        assert_eq!(a.current_frame(), SWEEP_FRAMES - 1);
    }

    #[test]
    fn playback_targets_six_fps_without_drift() {
        // 6 FPS == ~166.7 ms per frame.
        assert_eq!(SWEEP_INTERVAL_MS, 167);
        let mut a = Animator::new(AnimationSet::Sweep);
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
        let mut a = Animator::new(AnimationSet::Sweep);
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
        assert_eq!(Animator::frame_at_tick(0, 8), 0);
        assert_eq!(Animator::frame_at_tick(8, 8), 0);
        assert_eq!(Animator::frame_at_tick(9, 8), 1);
    }

    #[test]
    fn idle_set_uses_its_own_frame_count() {
        assert_eq!(AnimationSet::Idle.frame_count(), IDLE_FRAMES);
        assert_eq!(AnimationSet::Idle.sprite_set(), SpriteSetSpec::IDLE);
        let a = Animator::new(AnimationSet::Idle);
        assert_eq!(a.set(), AnimationSet::Idle);
    }

    #[test]
    fn idle_ambient_loops_through_all_frames() {
        let interval = SWEEP_INTERVAL_MS * 3;
        let mut a = Animator::new(AnimationSet::Idle);
        a.set_mode(AnimationMode::Ambient);
        assert_eq!(a.ms_until_next_frame(0), interval);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(IDLE_FRAMES * 2) {
            t += interval;
            assert!(a.tick(t), "ambient idle frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), IDLE_FRAMES);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn health_set_uses_its_own_frame_count_and_frames() {
        assert_eq!(AnimationSet::Health.frame_count(), HEALTH_FRAMES);
        assert_eq!(AnimationSet::Health.sprite_set(), SpriteSetSpec::HEALTH);
        let a = Animator::new(AnimationSet::Health);
        assert_eq!(a.set(), AnimationSet::Health);
    }

    #[test]
    fn health_sweeping_loops_through_all_frames() {
        let mut a = Animator::new(AnimationSet::Health);
        a.set_mode(AnimationMode::Sweeping);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(HEALTH_FRAMES * 2) {
            t += SWEEP_INTERVAL_MS;
            assert!(a.tick(t), "health frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), HEALTH_FRAMES);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn about_set_uses_its_own_frame_count_and_frames() {
        assert_eq!(AnimationSet::About.frame_count(), ABOUT_FRAMES);
        assert_eq!(AnimationSet::About.sprite_set(), SpriteSetSpec::ABOUT);
        let a = Animator::new(AnimationSet::About);
        assert_eq!(a.set(), AnimationSet::About);
    }

    #[test]
    fn about_ambient_loops_through_all_frames() {
        let interval = SWEEP_INTERVAL_MS * 3;
        let mut a = Animator::new(AnimationSet::About);
        a.set_mode(AnimationMode::Ambient);
        assert_eq!(a.ms_until_next_frame(0), interval);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(ABOUT_FRAMES * 2) {
            t += interval;
            assert!(a.tick(t), "ambient about frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), ABOUT_FRAMES);
        assert_eq!(a.current_frame(), 0, "loop returns to the first frame");
    }

    #[test]
    fn help_set_uses_its_own_frame_count_and_frames() {
        assert_eq!(AnimationSet::Help.frame_count(), HELP_FRAMES);
        assert_eq!(AnimationSet::Help.sprite_set(), SpriteSetSpec::HELP);
        let a = Animator::new(AnimationSet::Help);
        assert_eq!(a.set(), AnimationSet::Help);
    }

    #[test]
    fn help_ambient_loops_through_all_frames() {
        let interval = SWEEP_INTERVAL_MS * 3;
        let mut a = Animator::new(AnimationSet::Help);
        a.set_mode(AnimationMode::Ambient);
        assert_eq!(a.ms_until_next_frame(0), interval);
        let mut seen = std::collections::HashSet::new();
        let mut t = 0u64;
        for _ in 0..(HELP_FRAMES * 2) {
            t += interval;
            assert!(a.tick(t), "ambient help frame should advance");
            seen.insert(a.current_frame());
        }
        assert_eq!(seen.len(), HELP_FRAMES);
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
        let mut a = Animator::new(AnimationSet::Sweep);
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
        let mut a = Animator::new(AnimationSet::Sweep);
        a.set_mode(AnimationMode::Sweeping);
        // Advance to the final sweep frame.
        a.tick(SWEEP_INTERVAL_MS * (SWEEP_FRAMES as u64 - 1));
        assert_eq!(a.current_frame(), SWEEP_FRAMES - 1);

        assert!(a.select_set(AnimationSet::Idle, 10_000));
        assert_eq!(a.current_frame(), 0);
        for step in 1..=(IDLE_FRAMES as u64 * 2) {
            a.tick(10_000 + step * SWEEP_INTERVAL_MS);
            assert!(a.current_frame() < AnimationSet::Idle.frame_count());
        }

        assert!(a.select_set(AnimationSet::Health, 20_000));
        assert_eq!(a.current_frame(), 0);
        for step in 1..=(HEALTH_FRAMES as u64 * 2) {
            a.tick(20_000 + step * SWEEP_INTERVAL_MS);
            assert!(a.current_frame() < AnimationSet::Health.frame_count());
        }

        assert!(a.select_set(AnimationSet::About, 30_000));
        assert_eq!(a.current_frame(), 0);
        for step in 1..=(ABOUT_FRAMES as u64 * 2) {
            a.tick(30_000 + step * SWEEP_INTERVAL_MS);
            assert!(a.current_frame() < AnimationSet::About.frame_count());
        }

        assert!(a.select_set(AnimationSet::Help, 35_000));
        assert_eq!(a.current_frame(), 0);
        for step in 1..=(HELP_FRAMES as u64 * 2) {
            a.tick(35_000 + step * SWEEP_INTERVAL_MS);
            assert!(a.current_frame() < AnimationSet::Help.frame_count());
        }

        assert!(a.select_set(AnimationSet::Sweep, 40_000));
        assert_eq!(a.current_frame(), 0);
        assert!(a.current_frame() < AnimationSet::Sweep.frame_count());
        assert!(!a.select_set(AnimationSet::Sweep, 41_000));
    }
}
