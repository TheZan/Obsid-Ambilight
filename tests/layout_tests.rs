use obsid_ambilight::ambilight::layout::{
    self, EDGE_INSET, HighlightTarget, MAX_SEGMENTS, SegmentError, default_counts, highlight_frame, led_points,
    perimeter, sanitize, trim,
};
use obsid_ambilight::config::Segment;
use obsid_ambilight::protocol::Rgb;

fn seg(from: usize, to: usize, start: [f64; 2], end: [f64; 2]) -> Segment {
    Segment { from, to, start, end }
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

#[test]
fn leds_are_spread_evenly_along_the_line() {
    let s = seg(0, 3, [0.0, 0.0], [1.0, 0.0]);
    assert_eq!(s.count(), 4);
    assert_eq!(s.leds(), vec![0, 1, 2, 3]);
    assert!(close(s.point(0), [0.125, 0.0]));
    assert!(close(s.point(3), [0.875, 0.0]));
}

#[test]
fn reversed_indices_run_backwards() {
    let s = seg(9, 6, [0.0, 0.5], [0.0, 0.1]);
    assert_eq!(s.leds(), vec![9, 8, 7, 6]);
    let points = led_points(&[s], 10);
    assert!(close(points[9].unwrap(), [0.0, 0.45]), "LED `from` sits at the start");
    assert!(close(points[6].unwrap(), [0.0, 0.15]));
    assert!(points[0].is_none(), "LEDs outside every segment have no point");
}

#[test]
fn single_led_segment_sits_in_the_middle() {
    let s = seg(5, 5, [0.2, 0.2], [0.4, 0.6]);
    assert!(close(s.point(0), [0.3, 0.4]));
}

#[test]
fn reversing_swaps_the_order_but_keeps_the_line() {
    let s = seg(0, 9, [0.1, 0.2], [0.3, 0.4]);
    assert_eq!(s.reversed(), seg(9, 0, [0.1, 0.2], [0.3, 0.4]));
}

#[test]
fn check_flags_indices_and_points() {
    assert_eq!(layout::check(&seg(0, 59, [0.0, 0.0], [1.0, 1.0]), 60), Ok(()));
    assert_eq!(layout::check(&seg(0, 60, [0.0, 0.0], [1.0, 1.0]), 60), Err(SegmentError::OutOfRange { led_count: 60 }));
    assert_eq!(layout::check(&seg(0, 5, [1.5, 0.0], [1.0, 1.0]), 60), Err(SegmentError::BadPoint));
    assert_eq!(layout::check(&seg(0, 5, [f64::NAN, 0.0], [1.0, 1.0]), 60), Err(SegmentError::BadPoint));
}

#[test]
fn errors_point_at_the_earlier_overlapping_segment() {
    let segs =
        [seg(0, 9, [0.0, 0.0], [1.0, 0.0]), seg(20, 25, [0.0, 0.0], [1.0, 0.0]), seg(12, 5, [0.0, 0.0], [1.0, 0.0])];
    assert_eq!(layout::errors(&segs, 60), vec![None, None, Some(SegmentError::Overlap(0))]);
}

#[test]
fn sanitize_drops_bad_overlapping_and_extra_segments() {
    let mut input = vec![
        seg(0, 9, [0.0, 0.0], [1.0, 0.0]),
        seg(5, 12, [0.0, 0.0], [1.0, 0.0]),
        seg(130, 131, [0.0, 0.0], [1.0, 0.0]),
    ];
    for i in 0..20 {
        input.push(seg(20 + i * 2, 21 + i * 2, [0.0, 0.0], [1.0, 0.0]));
    }
    let (kept, warnings) = sanitize(input, 120);
    assert_eq!(kept.len(), MAX_SEGMENTS);
    assert_eq!(kept[0], seg(0, 9, [0.0, 0.0], [1.0, 0.0]));
    assert!(warnings.iter().any(|w| w.contains("segments[1]")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("segments[2]")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("only the first 16")), "{warnings:?}");
}

#[test]
fn perimeter_runs_clockwise_from_the_bottom_left() {
    let p = perimeter([11, 19, 11, 19]);
    let (a, b) = (EDGE_INSET, 1.0 - EDGE_INSET);
    assert_eq!(
        p,
        vec![
            seg(0, 10, [a, b], [a, a]),
            seg(11, 29, [a, a], [b, a]),
            seg(30, 40, [b, a], [b, b]),
            seg(41, 59, [b, b], [a, b]),
        ]
    );
    assert_eq!(perimeter([0, 5, 0, 0]), vec![seg(0, 4, [a, a], [b, a])], "empty sides are skipped");
}

#[test]
fn default_counts_follow_16_by_9_and_sum_up() {
    assert_eq!(default_counts(60), [11, 19, 11, 19]);
    for n in 1..=120 {
        assert_eq!(default_counts(n).iter().sum::<usize>(), n, "n = {n}");
    }
    assert_eq!(layout::default_layout(60), perimeter([11, 19, 11, 19]));
}

#[test]
fn trim_keeps_led_positions() {
    let fwd = seg(0, 9, [0.0, 0.0], [1.0, 0.0]);
    let rev = seg(19, 10, [0.0, 1.0], [1.0, 1.0]);
    let beyond = seg(30, 35, [0.0, 0.5], [1.0, 0.5]);
    let before = led_points(&[fwd.clone(), rev.clone()], 40);
    let t = trim(&[fwd, rev, beyond], 15);
    assert_eq!(t.len(), 2, "segments starting beyond the count are dropped");
    assert_eq!((t[0].from, t[0].to), (0, 9));
    assert_eq!((t[1].from, t[1].to), (14, 10));
    let after = led_points(&t, 15);
    for i in 0..15 {
        match (before[i], after[i]) {
            (Some(x), Some(y)) => assert!(close(x, y), "LED {i}: {x:?} vs {y:?}"),
            (x, y) => assert_eq!(x.is_some(), y.is_some(), "LED {i}"),
        }
    }
}

#[test]
fn led_highlight_marks_direction() {
    let f = highlight_frame(HighlightTarget::Led(12), &[], 30);
    assert_eq!(f.len(), 30);
    assert_eq!(f[12], Rgb::new(255, 255, 255));
    assert_eq!(f[11], Rgb::new(48, 0, 0), "before: dim red");
    assert_eq!(f[2], Rgb::new(48, 0, 0));
    assert_eq!(f[1], Rgb::BLACK);
    assert_eq!(f[13], Rgb::new(0, 48, 0), "after: dim green");
    assert_eq!(f[22], Rgb::new(0, 48, 0));
    assert_eq!(f[23], Rgb::BLACK);
}

#[test]
fn segment_highlight_is_a_green_to_blue_gradient() {
    let segs = [seg(0, 4, [0.0, 0.0], [1.0, 0.0]), seg(9, 5, [0.0, 1.0], [1.0, 1.0])];
    let f = highlight_frame(HighlightTarget::Segment(1), &segs, 12);
    assert_eq!(f[9], Rgb::new(0, 255, 0), "`from` is green");
    assert_eq!(f[5], Rgb::new(0, 0, 255), "`to` is blue");
    assert_eq!(f[0], Rgb::BLACK, "other segments are dark");
    assert_eq!(highlight_frame(HighlightTarget::Segment(7), &segs, 12), vec![Rgb::BLACK; 12]);
}
