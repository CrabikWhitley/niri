//! angel-niri: the background doesn't get in a game's way.
//!
//! `niri msg action set-background-fps N --output GAME` (a game mode script sends it while a game
//! runs, 0 when it ends): every output but the game's redraws at most N times a second. Its windows
//! get their frame callbacks as rarely, so they draw less too, and the GPU is left to the game.
//! Only the moment of a redraw is put off: the redraw state machine is untouched.
//!
//! Never put off: the game's output (named once, so moving the focus to another screen mid-game
//! changes nothing) and any output a screencast shows — itself or a window on it — so a stream or
//! a recording never loses frames.
//!
//! `niri msg action set-tearing --output GAME [--on false]` (the same script, when the game mode
//! asks for it): while a fullscreen window is on top of the game's output, a frame that changes
//! only the primary plane's buffer — the game scanned out, or composited with nothing else moving —
//! is flipped at once instead of at the next vblank (angel-smithay's async flips): less input lag,
//! at the price of tearing. The desktop never tears.

use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

#[cfg(feature = "xdp-gnome-screencast")]
use crate::niri::CastTarget;
use crate::niri::Niri;
use crate::utils::get_monotonic_time;

impl Niri {
    /// `keep`: the game's output; none given, the active one now.
    pub fn set_background_fps(&mut self, fps: u32, keep: Option<String>) {
        self.background_fps = (fps > 0).then_some(fps);
        self.background_keep = keep.or_else(|| self.layout.active_output().map(|o| o.name()));
        // (outputs put off until now are let go on the next loop)
        for state in self.output_state.values_mut() {
            state.background_until = None;
        }
    }

    /// `output`: the game's; none given, the active one now.
    pub fn set_tearing(&mut self, on: bool, output: Option<String>) {
        self.tearing_output = on
            .then(|| output.or_else(|| self.layout.active_output().map(|o| o.name())))
            .flatten();
    }

    /// Whether `output`'s frames may tear now: it is the game's and shows a fullscreen window.
    pub fn tearing_now(&self, output: &Output) -> bool {
        self.tearing_output.as_deref() == Some(output.name().as_str())
            && self
                .layout
                .monitor_for_output(output)
                .is_some_and(|mon| mon.fullscreen_window_above_top_layer().is_some())
    }

    /// Whether `output` has to wait before its queued redraw. Arms a timer that lets it go.
    pub fn background_wait(&mut self, output: &Output) -> bool {
        let Some(fps) = self.background_fps else {
            return false;
        };
        if self.background_keep.as_deref() == Some(output.name().as_str()) || self.is_cast(output) {
            return false;
        }
        let now = get_monotonic_time();
        let state = self.output_state.get_mut(output).unwrap();
        if let Some(until) = state.background_until {
            return now < until;
        }
        let Some(last) = state.last_redraw_at else {
            return false;
        };
        let next = last + Duration::from_secs(1) / fps;
        if now >= next {
            return false;
        }
        state.background_until = Some(next);
        let output = output.clone();
        self.event_loop
            .insert_source(Timer::from_duration(next - now), move |_, _, state| {
                if let Some(s) = state.niri.output_state.get_mut(&output) {
                    s.background_until = None;
                }
                TimeoutAction::Drop
            })
            .unwrap();
        true
    }

    /// Whether a screencast shows this output, or a window on it.
    #[cfg(not(feature = "xdp-gnome-screencast"))]
    fn is_cast(&self, _output: &Output) -> bool {
        false
    }

    /// Whether a screencast shows this output, or a window on it.
    #[cfg(feature = "xdp-gnome-screencast")]
    fn is_cast(&self, output: &Output) -> bool {
        self.casting.casts.iter().any(|cast| match &cast.target {
            CastTarget::Nothing => false,
            CastTarget::Output { name, .. } => *name == output.name(),
            CastTarget::Window { id } => self.layout.windows().any(|(mon, m)| {
                m.id().get() == *id && mon.is_some_and(|mon| mon.output() == output)
            }),
        })
    }
}
