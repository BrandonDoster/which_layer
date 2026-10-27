//! When the overlay is on screen and how opaque it is. Pure logic, no window:
//! the app feeds it events and the current time, then mirrors `on_screen()`
//! and `alpha()` onto the real window, and sleeps until `next_wake()`.
//!
//! Showing fades in quickly (never jumpy, never laggy). Hiding after a layer
//! change first dwells on screen for `hide_delay`, then fades out; a manual
//! hide skips the dwell. Anything that shows again mid-dwell or mid-fade picks
//! up from the current opacity.
//!
//! A toggle popup (Caps Lock ON, ...) is drawn over the keyboard if that's on
//! screen; otherwise it comes up alone ("popup only") for its duration. Leaving
//! a layer while a popup is up keeps the popup, alone, until it's done.

use std::time::{Duration, Instant};

/// Repaint interval while fading.
pub const FRAME: Duration = Duration::from_millis(16);
/// Upper bound on any configured duration, so a typo can't freeze the overlay.
const MAX_DURATION: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub fade_in: Duration,
    pub fade_out: Duration,
    /// Dwell before an automatic fade-out starts.
    pub hide_delay: Duration,
    /// Hide an auto-shown overlay after this long even if the layer stays on.
    pub auto_hide: Option<Duration>,
}

impl Timing {
    pub fn from_millis(fade_in: u32, fade_out: u32, hide_delay: u32, auto_hide_secs: f32) -> Timing {
        let ms = |v: u32| Duration::from_millis(v as u64).min(MAX_DURATION);
        let auto_hide = (auto_hide_secs.is_finite() && auto_hide_secs > 0.0)
            .then(|| Duration::from_secs_f32(auto_hide_secs.min(3600.0)));
        Timing { fade_in: ms(fade_in), fade_out: ms(fade_out), hide_delay: ms(hide_delay), auto_hide }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Hidden,
    FadingIn { from: f32, start: Instant },
    Shown,
    Dwell { until: Instant },
    FadingOut { from: f32, start: Instant },
}

pub struct Visibility {
    phase: Phase,
    timing: Timing,
    pub pinned: bool,
    pub auto_show: bool,
    /// On screen because of a layer change rather than the user.
    auto_shown: bool,
    auto_hide_at: Option<Instant>,
    popup_until: Option<Instant>,
    /// On screen only for a popup: draw it without the keyboard.
    popup_only: bool,
}

impl Visibility {
    pub fn new(timing: Timing, pinned: bool, auto_show: bool) -> Visibility {
        let mut v = Visibility {
            phase: Phase::Hidden,
            timing,
            pinned,
            auto_show,
            auto_shown: false,
            auto_hide_at: None,
            popup_until: None,
            popup_only: false,
        };
        if pinned {
            v.phase = Phase::Shown;
        }
        v
    }

    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// The window should exist on screen (including dwell and fade-out).
    pub fn on_screen(&self) -> bool {
        self.phase != Phase::Hidden
    }

    /// Shown or on its way in; a hide is not in progress.
    pub fn showing(&self) -> bool {
        matches!(self.phase, Phase::Shown | Phase::FadingIn { .. })
    }

    /// Opacity multiplier, 0..=1 (times the user's opacity setting).
    pub fn alpha(&self, now: Instant) -> f32 {
        match self.phase {
            Phase::Hidden => 0.0,
            Phase::Shown | Phase::Dwell { .. } => 1.0,
            Phase::FadingIn { from, start } => from + (1.0 - from) * progress(now, start, self.timing.fade_in),
            Phase::FadingOut { from, start } => from * (1.0 - progress(now, start, self.timing.fade_out)),
        }
    }

    /// New QMK layer_state from the keyboard.
    pub fn layer_state(&mut self, state: u32, now: Instant) {
        if self.pinned {
            return;
        }
        if state > 1 {
            if self.auto_show && (!self.showing() || self.auto_shown) {
                self.show(now);
                self.auto_shown = true;
                self.auto_hide_at = self.timing.auto_hide.map(|d| now + d);
            }
        } else if self.auto_shown {
            match self.popup_until.filter(|&t| t > now) {
                // Keep the popup up for its full time, without the keyboard.
                Some(t) => {
                    self.popup_only = true;
                    self.auto_hide_at = Some(t);
                }
                None => self.hide(now, true),
            }
        }
    }

    /// A toggle changed: show its popup for `duration`. Over the keyboard if
    /// that's up (pinned, user-shown or a layer); else alone, then hides itself.
    pub fn popup(&mut self, now: Instant, duration: Duration) {
        let until = now + duration.min(MAX_DURATION);
        self.popup_until = Some(until);
        if !self.showing() || self.popup_only {
            self.show(now);
            self.popup_only = true;
            self.auto_shown = true;
            self.auto_hide_at = Some(until);
        }
    }

    /// Draw the popup now (still timing, or fading out on its own).
    pub fn popup_visible(&self, now: Instant) -> bool {
        self.popup_only || self.popup_until.is_some_and(|t| t > now)
    }

    /// Draw the keyboard (false while only a popup is up).
    pub fn keys_visible(&self) -> bool {
        !self.popup_only
    }

    /// Tray click / hotkey.
    pub fn toggle(&mut self, now: Instant) {
        if self.showing() {
            self.pinned = false; // hiding a pinned overlay unpins it
            self.hide(now, false);
        } else {
            self.show(now);
        }
    }

    pub fn set_pinned(&mut self, pinned: bool, now: Instant) {
        let was = self.pinned;
        self.pinned = pinned;
        if pinned {
            self.show(now);
        } else if was && !self.auto_shown {
            self.hide(now, false);
        }
    }

    /// Fades in from wherever the opacity is now. A user show clears any
    /// automatic hide; `layer_state` re-marks it as auto right after.
    pub fn show(&mut self, now: Instant) {
        self.auto_shown = false;
        self.auto_hide_at = None;
        self.popup_only = false;
        if !self.showing() {
            let from = self.alpha(now);
            self.phase = Phase::FadingIn { from, start: now };
            self.tick(now); // zero-length fade lands on Shown immediately
        }
    }

    /// `dwell`: wait `hide_delay` before fading (automatic hides only).
    pub fn hide(&mut self, now: Instant, dwell: bool) {
        self.auto_shown = false;
        self.auto_hide_at = None;
        if !self.showing() {
            return;
        }
        self.phase = if dwell && !self.timing.hide_delay.is_zero() {
            Phase::Dwell { until: now + self.timing.hide_delay }
        } else {
            Phase::FadingOut { from: self.alpha(now), start: now }
        };
        self.tick(now);
    }

    /// Advances fades, dwell and the auto-hide timer to `now`.
    pub fn tick(&mut self, now: Instant) {
        if self.auto_hide_at.is_some_and(|t| now >= t) {
            self.hide(now, true);
        }
        self.phase = match self.phase {
            Phase::FadingIn { start, .. } if progress(now, start, self.timing.fade_in) >= 1.0 => Phase::Shown,
            Phase::Dwell { until } if now >= until => Phase::FadingOut { from: 1.0, start: until },
            p => p,
        };
        if let Phase::FadingOut { start, .. } = self.phase {
            if progress(now, start, self.timing.fade_out) >= 1.0 {
                self.phase = Phase::Hidden;
            }
        }
    }

    /// When the app next needs to call `tick` (None = only on events).
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        let phase = match self.phase {
            Phase::FadingIn { .. } | Phase::FadingOut { .. } => Some(now + FRAME),
            Phase::Dwell { until } => Some(until),
            Phase::Hidden | Phase::Shown => None,
        };
        // A popup over the keyboard needs a repaint when it ends.
        let popup_end = self.popup_until.filter(|&t| t > now && self.on_screen());
        [phase, self.auto_hide_at, popup_end].into_iter().flatten().min()
    }
}

/// 0..=1 through a fade; a zero-length fade is already done.
fn progress(now: Instant, start: Instant, len: Duration) -> f32 {
    if len.is_zero() {
        return 1.0;
    }
    (now.saturating_duration_since(start).as_secs_f32() / len.as_secs_f32()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn timing() -> Timing {
        Timing::from_millis(100, 200, 300, 5.0)
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.02
    }

    #[test]
    fn auto_show_fades_in_and_dwells_before_fading_out() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        assert!(!v.on_screen());

        v.layer_state(0b10, t0);
        assert!(v.on_screen() && v.showing());
        assert!(approx(v.alpha(t0), 0.0));
        assert!(approx(v.alpha(t0 + 50 * MS), 0.5));
        v.tick(t0 + 100 * MS);
        assert!(approx(v.alpha(t0 + 100 * MS), 1.0));

        let t1 = t0 + 1000 * MS;
        v.layer_state(0b1, t1); // back to base: dwell first
        assert!(v.on_screen() && !v.showing());
        v.tick(t1 + 299 * MS);
        assert!(approx(v.alpha(t1 + 299 * MS), 1.0), "still dwelling");
        v.tick(t1 + 400 * MS);
        assert!(approx(v.alpha(t1 + 400 * MS), 0.5), "halfway through fade-out");
        v.tick(t1 + 500 * MS);
        assert!(!v.on_screen());
    }

    #[test]
    fn layer_again_during_dwell_or_fade_comes_straight_back() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        v.layer_state(0b10, t0);
        v.tick(t0 + 100 * MS);
        v.layer_state(0b1, t0 + 200 * MS);
        v.tick(t0 + 600 * MS); // 100ms into the fade-out, alpha 0.5
        v.layer_state(0b100, t0 + 600 * MS);
        assert!(v.showing());
        assert!(approx(v.alpha(t0 + 600 * MS), 0.5), "fades in from current opacity");
        assert!(approx(v.alpha(t0 + 650 * MS), 0.75));
        v.tick(t0 + 700 * MS);
        assert!(approx(v.alpha(t0 + 700 * MS), 1.0));
    }

    #[test]
    fn auto_hide_timer_hides_a_layer_left_on() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        v.layer_state(0b100, t0);
        assert_eq!(v.next_wake(t0), Some(t0 + FRAME));
        v.tick(t0 + 100 * MS);
        assert_eq!(v.next_wake(t0 + 100 * MS), Some(t0 + 5000 * MS));
        v.tick(t0 + 5000 * MS);
        assert!(!v.showing(), "timer started the dwell");
        v.tick(t0 + 5500 * MS);
        assert!(!v.on_screen());
    }

    #[test]
    fn user_shown_overlay_ignores_layer_changes() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        v.toggle(t0);
        v.layer_state(0b10, t0);
        v.layer_state(0b1, t0);
        assert!(v.showing());
        v.tick(t0 + 100 * MS);
        assert_eq!(v.next_wake(t0 + 100 * MS), None, "no auto-hide timer");
    }

    #[test]
    fn manual_hide_skips_the_dwell() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        v.toggle(t0);
        v.tick(t0 + 100 * MS);
        v.toggle(t0 + 100 * MS);
        assert!(approx(v.alpha(t0 + 200 * MS), 0.5));
    }

    #[test]
    fn pinned_stays_up_and_unpinning_hides() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), true, true);
        assert!(v.showing());
        v.layer_state(0b1, t0);
        assert!(v.showing());
        v.set_pinned(false, t0);
        assert!(!v.showing());
        v.set_pinned(true, t0);
        v.toggle(t0);
        assert!(!v.pinned && !v.showing(), "hiding unpins");
    }

    #[test]
    fn auto_show_off_never_shows_on_layers() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, false);
        v.layer_state(0b10, t0);
        assert!(!v.on_screen());
    }

    #[test]
    fn zero_durations_are_instant() {
        let t0 = Instant::now();
        let mut v = Visibility::new(Timing::from_millis(0, 0, 0, 0.0), false, true);
        v.layer_state(0b10, t0);
        assert!(approx(v.alpha(t0), 1.0));
        v.layer_state(0b1, t0);
        assert!(!v.on_screen());
        assert_eq!(v.next_wake(t0), None);
    }

    #[test]
    fn popup_alone_shows_then_hides_itself() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        v.popup(t0, 1000 * MS);
        assert!(v.showing() && !v.keys_visible() && v.popup_visible(t0));
        v.tick(t0 + 999 * MS);
        assert!(v.showing());
        v.tick(t0 + 1000 * MS);
        assert!(!v.showing() && v.on_screen(), "dwell then fade, like a layer");
        assert!(v.popup_visible(t0 + 1100 * MS), "still drawn while fading out");
        v.tick(t0 + 1500 * MS);
        assert!(!v.on_screen());
    }

    #[test]
    fn popup_over_the_keyboard_and_layer_changes() {
        let t0 = Instant::now();
        let mut v = Visibility::new(timing(), false, true);
        v.layer_state(0b100, t0); // holding a layer...
        v.popup(t0 + 10 * MS, 1000 * MS); // ...and toggling SOCD on it
        assert!(v.keys_visible() && v.popup_visible(t0 + 10 * MS));
        v.tick(t0 + 150 * MS); // faded in
        assert_eq!(v.next_wake(t0 + 150 * MS), Some(t0 + 1010 * MS), "repaint when the popup ends");
        v.layer_state(0b1, t0 + 200 * MS); // let go of the layer: popup stays, alone
        assert!(v.showing() && !v.keys_visible());
        v.tick(t0 + 1010 * MS);
        assert!(!v.showing(), "then hides at the popup's end");

        // A layer during a lone popup brings the keyboard back.
        let t1 = t0 + 5000 * MS;
        v.popup(t1, 1000 * MS);
        v.layer_state(0b10, t1 + 100 * MS);
        assert!(v.keys_visible() && v.popup_visible(t1 + 100 * MS));
        assert!(!v.popup_visible(t1 + 1000 * MS), "popup over the keyboard ends on time");

        // Pinned: the popup is just drawn on top; nothing hides.
        let mut p = Visibility::new(timing(), true, true);
        p.popup(t0, 1000 * MS);
        p.tick(t0 + 2000 * MS);
        assert!(p.showing() && p.keys_visible() && !p.popup_visible(t0 + 2000 * MS));
    }

    #[test]
    fn absurd_timings_are_clamped() {
        let t = Timing::from_millis(u32::MAX, u32::MAX, u32::MAX, f32::NAN);
        assert_eq!(t.fade_in, MAX_DURATION);
        assert_eq!(t.auto_hide, None);
        assert_eq!(Timing::from_millis(0, 0, 0, -1.0).auto_hide, None);
    }
}
