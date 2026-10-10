use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::Image;
use obsid_ambilight::ambilight::letterbox::{Bars, HOLD};
use obsid_ambilight::ambilight::pipeline::{Pipeline, Settings};
use obsid_ambilight::ambilight::smooth::{Smoother, tau};
use obsid_ambilight::config::{Config, Segment};
use obsid_ambilight::protocol::Rgb;

fn settings() -> Settings {
    let mut s = Settings::from_config(&Config::default());
    s.smoothing = 0;
    s
}

#[test]
fn settings_come_from_the_config() {
    let c = Config::default();
    let s = Settings::from_config(&c);
    assert_eq!(s.led_count, 60);
    assert_eq!(s.segments, c.ambilight.segments);
    assert_eq!(s.smoothing, 25);
    assert!(s.letterbox);
}

#[test]
fn a_red_screen_makes_every_led_red() {
    let mut p = Pipeline::new();
    let out = p.process(&Image::filled(160, 90, [1.0, 0.0, 0.0]), &settings(), Instant::now());
    assert_eq!(out, vec![Rgb::new(255, 0, 0); 60]);
}

#[test]
fn each_side_takes_its_own_colour() {
    let mut img = Image::filled(160, 90, [0.0, 0.0, 1.0]);
    for y in 0..90 {
        for x in 80..160 {
            img.set(x, y, [1.0, 0.0, 0.0]);
        }
    }
    let mut s = settings();
    s.led_count = 4;
    s.segments = vec![
        Segment { from: 0, to: 1, start: [0.02, 0.2], end: [0.02, 0.8] },
        Segment { from: 2, to: 3, start: [0.98, 0.2], end: [0.98, 0.8] },
    ];
    let out = Pipeline::new().process(&img, &s, Instant::now());
    assert_eq!(out, vec![Rgb::new(0, 0, 255), Rgb::new(0, 0, 255), Rgb::new(255, 0, 0), Rgb::new(255, 0, 0)]);
}

#[test]
fn leds_outside_segments_stay_dark() {
    let mut s = settings();
    s.segments = vec![Segment { from: 0, to: 9, start: [0.1, 0.1], end: [0.9, 0.1] }];
    let out = Pipeline::new().process(&Image::filled(160, 90, [1.0; 3]), &s, Instant::now());
    assert!(out[..10].iter().all(|c| *c == Rgb::new(255, 255, 255)));
    assert!(out[10..].iter().all(|c| *c == Rgb::BLACK));
}

#[test]
fn letterbox_moves_sampling_inside_the_picture() {
    // 21:9 picture: black bars of 11 rows, red picture between them.
    let mut img = Image::filled(160, 90, [0.0; 3]);
    for y in 11..79 {
        for x in 0..160 {
            img.set(x, y, [1.0, 0.0, 0.0]);
        }
    }
    let mut s = settings();
    s.led_count = 1;
    s.segments = vec![Segment { from: 0, to: 0, start: [0.5, 0.0], end: [0.5, 0.0] }];
    let t0 = Instant::now();
    let mut p = Pipeline::new();
    assert_eq!(p.process(&img, &s, t0), vec![Rgb::BLACK], "bars are not trusted yet");
    assert_eq!(p.process(&img, &s, t0 + HOLD), vec![Rgb::new(255, 0, 0)]);
    assert_eq!(p.bars(), Bars { vertical: 11, horizontal: 0 });

    s.letterbox = false;
    assert_eq!(p.process(&img, &s, t0 + HOLD * 2), vec![Rgb::BLACK]);
    assert_eq!(p.bars(), Bars::default(), "disabled detection reports no bars");
}

#[test]
fn smoothing_eases_small_changes() {
    let t0 = Instant::now();
    let mut sm = Smoother::new();
    assert_eq!(sm.step(&[[0.5; 3]], tau(25), t0), &[[0.5; 3]], "first frame is taken as is");
    let next = sm.step(&[[0.6; 3]], tau(25), t0 + Duration::from_millis(40))[0][0];
    assert!(next > 0.5 && next < 0.6, "{next}");
    assert_eq!(sm.step(&[[0.7; 3]], tau(0), t0 + Duration::from_millis(80)), &[[0.7; 3]], "0 % = no smoothing");
}

#[test]
fn scene_cuts_skip_smoothing() {
    let t0 = Instant::now();
    let mut sm = Smoother::new();
    sm.step(&[[0.0; 3]], tau(100), t0);
    assert_eq!(sm.step(&[[1.0; 3]], tau(100), t0 + Duration::from_millis(40)), &[[1.0; 3]]);
}

#[test]
fn tau_spans_zero_to_half_a_second() {
    assert_eq!(tau(0), Duration::ZERO);
    assert_eq!(tau(100), Duration::from_millis(500));
    assert_eq!(tau(25), Duration::from_millis(125));
}

#[test]
fn a_new_frame_size_resets_letterbox_state() {
    let mut img = Image::filled(160, 90, [0.0; 3]);
    for y in 11..79 {
        for x in 0..160 {
            img.set(x, y, [1.0, 0.0, 0.0]);
        }
    }
    let s = settings();
    let t0 = Instant::now();
    let mut p = Pipeline::new();
    p.process(&img, &s, t0);
    p.process(&img, &s, t0 + HOLD);
    assert_eq!(p.bars().vertical, 11);
    p.process(&Image::filled(120, 68, [0.5; 3]), &s, t0 + HOLD * 2);
    assert_eq!(p.bars(), Bars::default());
}
