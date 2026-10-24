//! Shake the mouse to find the cursor (crabikOS).
//!
//! Works like the shake cursor effect of KDE Plasma: shaking the mouse makes the cursor grow,
//! every next shake while it is big makes it grow some more, and a moment after the last shake
//! it shrinks back. While it is big, the theme's arrow is drawn instead of the current cursor,
//! loaded at a bigger size so it stays sharp.
//!
//! The shake detector is a port of KWin's `ShakeDetector`
//! (src/plugins/shakecursor/shakedetector.cpp, SPDX-FileCopyrightText: 2023 Vlad Zahorodnii
//! <vlad.zahorodnii@kde.org>, SPDX-License-Identifier: GPL-2.0-or-later), and the grow/shrink
//! behavior follows its `ShakeCursorEffect`.

use std::collections::VecDeque;
use std::time::Duration;

use niri_config::ShakeToFind;
use smithay::utils::{Logical, Point};

use crate::animation::{Animation, Clock, CubicBezier, Curve};

/// A movement whose bounding box diagonal is shorter than this never counts as a shake.
const MIN_DIAGONAL: f64 = 100.;

#[derive(Debug, Clone, Copy)]
struct HistoryItem {
    pos: Point<f64, Logical>,
    time: Duration,
}

/// Tells a shake apart from ordinary movement.
///
/// Keeps the pointer positions of the last `interval`. Consecutive moves in the same direction
/// are merged into one segment, so the history only holds the turning points. The movement is a
/// shake when its path is much longer than the diagonal of its bounding box: back and forth over
/// the same small area.
#[derive(Debug)]
pub struct ShakeDetector {
    history: VecDeque<HistoryItem>,
    interval: Duration,
    sensitivity: f64,
}

impl ShakeDetector {
    pub fn new(interval: Duration, sensitivity: f64) -> Self {
        Self {
            history: VecDeque::new(),
            interval,
            sensitivity,
        }
    }

    pub fn set_params(&mut self, interval: Duration, sensitivity: f64) {
        self.interval = interval;
        self.sensitivity = sensitivity;
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }

    /// Feeds a new pointer position. Returns `true` when the movement became a shake.
    pub fn update(&mut self, pos: Point<f64, Logical>, time: Duration) -> bool {
        // Forget the movement older than the interval.
        while let Some(front) = self.history.front() {
            if time.saturating_sub(front.time) < self.interval {
                break;
            }
            self.history.pop_front();
        }

        // Still going the same way: extend the last segment instead of adding a point.
        let len = self.history.len();
        if len >= 2 {
            let last = self.history[len - 1];
            let prev = self.history[len - 2];
            if same_sign(last.pos.x - prev.pos.x, pos.x - last.pos.x)
                && same_sign(last.pos.y - prev.pos.y, pos.y - last.pos.y)
            {
                self.history[len - 1] = HistoryItem { pos, time };
                return false;
            }
        }

        self.history.push_back(HistoryItem { pos, time });

        let first = self.history[0].pos;
        let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
        let mut distance = 0.;
        for (a, b) in self.history.iter().zip(self.history.iter().skip(1)) {
            distance += (b.pos.x - a.pos.x).hypot(b.pos.y - a.pos.y);
            left = left.min(b.pos.x);
            top = top.min(b.pos.y);
            right = right.max(b.pos.x);
            bottom = bottom.max(b.pos.y);
        }

        let diagonal = (right - left).hypot(bottom - top);
        if diagonal < MIN_DIAGONAL {
            return false;
        }

        if distance / diagonal > self.sensitivity {
            self.history.clear();
            return true;
        }

        false
    }
}

/// Whether two moves go the same way along one axis (with a pixel of tolerance for jitter).
fn same_sign(a: f64, b: f64) -> bool {
    const TOLERANCE: f64 = 1.;
    (a >= -TOLERANCE && b >= -TOLERANCE) || (a <= TOLERANCE && b <= TOLERANCE)
}

/// The detector plus the cursor magnification it drives.
#[derive(Debug)]
pub struct CursorShake {
    config: ShakeToFind,
    detector: ShakeDetector,
    clock: Clock,
    /// Grow or shrink in progress (or finished at `target`).
    anim: Option<Animation>,
    /// The magnification the cursor is at or is going to (1 = normal size).
    target: f64,
    /// When to start shrinking back, in clock time.
    deflate_at: Option<Duration>,
}

impl CursorShake {
    pub fn new(clock: Clock, config: ShakeToFind) -> Self {
        Self {
            detector: ShakeDetector::new(
                Duration::from_millis(u64::from(config.interval_ms)),
                config.sensitivity,
            ),
            config,
            clock,
            anim: None,
            target: 1.,
            deflate_at: None,
        }
    }

    pub fn update_config(&mut self, config: ShakeToFind) {
        self.config = config;
        self.detector.set_params(
            Duration::from_millis(u64::from(config.interval_ms)),
            config.sensitivity,
        );
        if config.off {
            self.detector.reset();
            self.anim = None;
            self.target = 1.;
            self.deflate_at = None;
        }
    }

    /// Feeds pointer motion. Returns `true` if the cursor started growing.
    pub fn on_motion(&mut self, pos: Point<f64, Logical>, time: Duration) -> bool {
        if self.config.off {
            return false;
        }

        if self.detector.update(pos, time) {
            self.inflate();
            return true;
        }

        false
    }

    /// Movement that must not count (a button is held, the pointer is constrained…).
    pub fn interrupt(&mut self) {
        self.detector.reset();
    }

    /// Grows the cursor right away, as if the mouse was shaken (the `find-cursor` action).
    ///
    /// Works even when shaking is turned off: it was asked for explicitly.
    pub fn find(&mut self) {
        self.inflate();
    }

    fn inflate(&mut self) {
        let magnification = if self.target == 1. {
            self.config.magnification
        } else {
            self.target + self.config.over_magnification
        };
        let max = self.config.max_magnification.max(1.);
        self.animate_to(magnification.clamp(1., max));
        self.deflate_at =
            Some(self.clock.now() + Duration::from_millis(u64::from(self.config.hold_ms)));
    }

    fn animate_to(&mut self, magnification: f64) {
        if self.target == magnification {
            return;
        }

        let from = self.magnification();
        // InOutCubic, like KWin.
        let curve = Curve::CubicBezier(CubicBezier::new(0.65, 0., 0.35, 1.));
        self.anim = Some(Animation::ease(
            self.clock.clone(),
            from,
            magnification,
            0.,
            u64::from(self.config.animation_ms),
            curve,
        ));
        self.target = magnification;
    }

    /// Starts shrinking once the hold time is over. Call before every frame.
    pub fn advance(&mut self) {
        if let Some(at) = self.deflate_at {
            if self.clock.now() >= at {
                self.deflate_at = None;
                self.animate_to(1.);
            }
        }

        if self.target == 1. && self.anim.as_ref().is_some_and(|anim| anim.is_done()) {
            self.anim = None;
        }
    }

    /// Current magnification; 1 when the cursor is its normal size.
    pub fn magnification(&self) -> f64 {
        match &self.anim {
            Some(anim) => anim.value().max(1.),
            None => self.target,
        }
    }

    /// The scale to load the big cursor at, so that it stays sharp at its biggest.
    pub fn load_scale(&self) -> i32 {
        self.config.max_magnification.clamp(1., 8.).ceil() as i32
    }

    /// Whether frames must keep coming: the cursor is big or changing size.
    pub fn is_active(&self) -> bool {
        self.target != 1. || self.anim.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn detector() -> ShakeDetector {
        let config = ShakeToFind::default();
        ShakeDetector::new(
            Duration::from_millis(u64::from(config.interval_ms)),
            config.sensitivity,
        )
    }

    /// Moves along x between `from` and `to`, `steps` events per stroke, `step_ms` apart.
    fn strokes(
        d: &mut ShakeDetector,
        start: u64,
        count: usize,
        from: f64,
        to: f64,
        steps: usize,
        step_ms: u64,
    ) -> Option<u64> {
        let mut t = start;
        for i in 0..count {
            let (a, b) = if i % 2 == 0 { (from, to) } else { (to, from) };
            for s in 1..=steps {
                let x = a + (b - a) * s as f64 / steps as f64;
                t += step_ms;
                if d.update(Point::from((x, 500.)), ms(t)) {
                    return Some(t);
                }
            }
        }
        None
    }

    #[test]
    fn straight_movement_is_not_a_shake() {
        let mut d = detector();
        for i in 0..200 {
            assert!(!d.update(Point::from((i as f64 * 8., 300.)), ms(i * 4)));
        }
    }

    #[test]
    fn going_back_once_is_not_a_shake() {
        let mut d = detector();
        assert_eq!(strokes(&mut d, 0, 2, 100., 500., 20, 5), None);
    }

    #[test]
    fn fast_back_and_forth_is_a_shake() {
        let mut d = detector();
        // Six quick 300 px strokes in about half a second.
        assert!(strokes(&mut d, 0, 6, 300., 600., 10, 8).is_some());
    }

    #[test]
    fn tiny_jitter_is_not_a_shake() {
        let mut d = detector();
        // Back and forth over 40 px: the bounding box is under 100 px.
        assert_eq!(strokes(&mut d, 0, 30, 500., 540., 4, 8), None);
    }

    #[test]
    fn slow_strokes_are_forgotten() {
        let mut d = detector();
        // One stroke a second: the history never holds more than a stroke or two.
        assert_eq!(strokes(&mut d, 0, 8, 300., 600., 10, 100), None);
    }

    #[test]
    fn grows_holds_and_shrinks_back() {
        let mut clock = Clock::with_time(Duration::ZERO);
        let config = ShakeToFind {
            off: false,
            ..ShakeToFind::default()
        };
        let mut shake = CursorShake::new(clock.clone(), config);
        assert!(!shake.is_active());

        shake.find();
        assert!(shake.is_active());
        clock.set_unadjusted(ms(300));
        shake.advance();
        assert_eq!(shake.magnification(), 3.);

        // Shaken again while big: one more step.
        shake.find();
        clock.set_unadjusted(ms(600));
        shake.advance();
        assert_eq!(shake.magnification(), 4.);

        // Stays big for hold_ms after the last shake (2300)…
        clock.set_unadjusted(ms(2250));
        shake.advance();
        assert_eq!(shake.magnification(), 4.);

        // …then shrinks within animation_ms and stops asking for frames.
        clock.set_unadjusted(ms(2350));
        shake.advance();
        assert!(shake.is_active());
        clock.set_unadjusted(ms(2700));
        shake.advance();
        assert_eq!(shake.magnification(), 1.);
        assert!(!shake.is_active());
    }

    #[test]
    fn never_grows_past_the_max() {
        let clock = Clock::with_time(Duration::ZERO);
        let config = ShakeToFind {
            off: false,
            max_magnification: 4.5,
            ..ShakeToFind::default()
        };
        let mut shake = CursorShake::new(clock, config);
        for _ in 0..10 {
            shake.find();
        }
        assert_eq!(shake.target, 4.5);
    }

    #[test]
    fn off_ignores_motion_but_not_find() {
        let clock = Clock::with_time(Duration::ZERO);
        let mut shake = CursorShake::new(clock, ShakeToFind::default());
        let mut d = detector();
        // The same shake that the detector alone would catch…
        assert!(strokes(&mut d, 0, 6, 300., 600., 10, 8).is_some());
        // …does nothing while the feature is off.
        let mut t = 0;
        for i in 0..60 {
            let x = if (i / 10) % 2 == 0 {
                300. + (i % 10) as f64 * 30.
            } else {
                600. - (i % 10) as f64 * 30.
            };
            t += 8;
            assert!(!shake.on_motion(Point::from((x, 500.)), ms(t)));
        }
        assert!(!shake.is_active());
        // The explicit action still works.
        shake.find();
        assert!(shake.is_active());
    }

    #[test]
    fn shake_clears_history() {
        let mut d = detector();
        let t = strokes(&mut d, 0, 6, 300., 600., 10, 8).unwrap();
        // Right after a shake, a single move can't be a shake again.
        assert!(!d.update(Point::from((310., 500.)), ms(t + 8)));
    }
}
