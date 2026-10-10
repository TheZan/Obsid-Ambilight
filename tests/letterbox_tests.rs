use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::{Image, Rect};
use obsid_ambilight::ambilight::letterbox::{Bars, HOLD, Letterbox, detect};

const W: usize = 160;
const H: usize = 90;

/// Black frame with a coloured box inside the given margins.
fn framed(top: usize, side: usize) -> Image {
    let mut img = Image::filled(W, H, [0.0; 3]);
    for y in top..H - top {
        for x in side..W - side {
            img.set(x, y, [0.5, 0.3, 0.2]);
        }
    }
    img
}

#[test]
fn cinema_bars_are_found() {
    assert_eq!(detect(&framed(11, 0)), Some(Bars { vertical: 11, horizontal: 0 }));
}

#[test]
fn pillarbox_bars_are_found() {
    assert_eq!(detect(&framed(0, 20)), Some(Bars { vertical: 0, horizontal: 20 }));
}

#[test]
fn a_dark_sky_is_not_a_bar() {
    let mut img = Image::filled(W, H, [0.5; 3]);
    for y in 0..30 {
        for x in 0..W {
            img.set(x, y, [0.0; 3]);
        }
    }
    assert_eq!(detect(&img), Some(Bars::default()), "only one side is dark");
}

#[test]
fn bars_wider_than_30_percent_are_ignored() {
    assert_eq!(detect(&framed(30, 0)), Some(Bars::default()));
}

#[test]
fn dim_content_is_not_black() {
    let img = Image::filled(W, H, [0.1, 0.1, 0.1]);
    assert_eq!(detect(&img), Some(Bars::default()));
}

#[test]
fn a_black_frame_says_nothing() {
    assert_eq!(detect(&Image::filled(W, H, [0.0; 3])), None);
}

#[test]
fn new_bars_apply_only_after_holding() {
    let t0 = Instant::now();
    let mut lb = Letterbox::new();
    let img = framed(11, 0);
    assert_eq!(lb.update(&img, t0), Bars::default());
    assert_eq!(lb.update(&img, t0 + HOLD / 2), Bars::default());
    assert_eq!(lb.update(&img, t0 + HOLD), Bars { vertical: 11, horizontal: 0 });
    // A black frame (fade) keeps the bars.
    assert_eq!(lb.update(&Image::filled(W, H, [0.0; 3]), t0 + HOLD * 2), Bars { vertical: 11, horizontal: 0 });
}

#[test]
fn a_flickering_candidate_restarts_the_timer() {
    let t0 = Instant::now();
    let mut lb = Letterbox::new();
    lb.update(&framed(11, 0), t0);
    lb.update(&framed(12, 0), t0 + Duration::from_millis(600));
    assert_eq!(lb.update(&framed(12, 0), t0 + Duration::from_millis(1200)), Bars::default());
    assert_eq!(lb.update(&framed(12, 0), t0 + Duration::from_millis(1600)), Bars { vertical: 12, horizontal: 0 });
}

#[test]
fn content_rect_excludes_the_bars() {
    let img = framed(11, 0);
    assert_eq!(Bars { vertical: 11, horizontal: 4 }.content(&img), Rect { x0: 4, y0: 11, x1: 156, y1: 79 });
}
