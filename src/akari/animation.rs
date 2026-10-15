//! Frame sequencing and timing. Future animation sets (idle/health/…)
//! plug in here without touching the TUI.

use crate::akari::sprites::{SWEEP_FRAMES, SWEEP_INTERVAL_MS};

/// Which animation set is displayed. Only Sweep ships in V00.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AnimationSet {
    Sweep,
    /// Reserved: shown as sweep fallback (never misrepresents status —
    /// the label always names the real operation).
    Idle,
}

/// Playback state driven by `App::execution_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationMode {
    /// Home/About/Help: gentle loop so the panel is alive but calm.
    Ambient,
    /// Cleanup actively running: full-speed sweep loop.
    Sweeping,
    /// Work finished: hold the final frame (not "still sweeping").
    Settled,
}

#[derive(Debug)]
#[allow(dead_code)]
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
            frame_count: SWEEP_FRAMES,
        }
    }

    pub fn set_mode(&mut self, mode: AnimationMode) {
        if self.mode != mode {
            self.mode = mode;
            if mode == AnimationMode::Settled {
                self.frame = self.frame_count.saturating_sub(1);
            }
        }
    }

    pub fn mode(&self) -> AnimationMode {
        self.mode
    }

    #[allow(dead_code)]
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
}
