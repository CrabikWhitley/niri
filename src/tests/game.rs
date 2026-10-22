use std::time::{Duration, Instant};

use super::*;

// angel-niri (game.rs): with a background limit, the game's output keeps its pace and the others
// redraw at most that often — also when the focus goes to another screen mid-game.
#[test]
fn background_outputs_redraw_at_most_fps() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.niri_focus_output(1);
    f.niri()
        .set_background_fps(20, Some("headless-1".to_owned()));
    // the focus leaves the game's screen: the game's output is still kept
    f.niri_focus_output(2);
    let outputs = [f.niri_output(1), f.niri_output(2)];

    let mut count = [0; 2];
    let mut last = [None; 2];
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        f.niri().queue_redraw_all();
        f.state.server.dispatch();
        for (i, output) in outputs.iter().enumerate() {
            let at = f.niri().output_state[output].last_redraw_at;
            if at != last[i] {
                count[i] += 1;
                last[i] = at;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    // 500 ms: the active output every loop, the other at 20 per second
    assert!(count[0] > 100, "active output redrew {} times", count[0]);
    assert!(
        (6..=12).contains(&count[1]),
        "background output redrew {} times",
        count[1]
    );

    // turned off: both keep up again
    f.niri().set_background_fps(0, None);
    let before = count[1];
    for _ in 0..20 {
        f.niri().queue_redraw_all();
        f.state.server.dispatch();
        let at = f.niri().output_state[&outputs[1]].last_redraw_at;
        if at != last[1] {
            count[1] += 1;
            last[1] = at;
        }
    }
    assert!(
        count[1] - before >= 15,
        "background output after the limit: {}",
        count[1] - before
    );
}

// angel-niri (game.rs): frames tear only on the game's output, and only while a fullscreen window
// is on top there.
#[test]
fn tearing_only_for_a_fullscreen_game() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.niri_focus_output(1);

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let (game, other) = (f.niri_output(1), f.niri_output(2));
    f.niri().set_tearing(true, Some("headless-1".to_owned()));
    assert!(
        !f.niri().tearing_now(&game),
        "a window not fullscreen tears"
    );

    let window_id = f.niri().layout.windows().next().unwrap().1.window.clone();
    f.niri().layout.set_fullscreen(&window_id, true);
    f.double_roundtrip(id);
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    // (not while the view still moves to it)
    f.niri_complete_animations();
    assert!(
        f.niri().tearing_now(&game),
        "the fullscreen game doesn't tear"
    );
    assert!(!f.niri().tearing_now(&other), "another output tears");

    f.niri().set_tearing(false, None);
    assert!(!f.niri().tearing_now(&game), "tearing stays on after off");
}
