use obsid_ambilight::ambilight::color::{Adjust, adjust, f16_to_f32, hdr_to_srgb, luma, srgb_encode, to_rgb};
use obsid_ambilight::ambilight::image::{Image, Rect};
use obsid_ambilight::config::AmbilightConfig;
use obsid_ambilight::protocol::Rgb;

const PLAIN: Adjust = Adjust { saturation: 1.0, brightness: 1.0, black_threshold: 0.0 };

fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

#[test]
fn sdr_white_in_hdr_becomes_full_white() {
    // 200-nit SDR white is 2.5 in scRGB (1.0 = 80 nits).
    assert_eq!(to_rgb(hdr_to_srgb([2.5; 3], 200.0)), Rgb::new(255, 255, 255));
    assert_eq!(to_rgb(hdr_to_srgb([1.25; 3], 200.0)), Rgb::new(188, 188, 188), "half linear = sRGB 188");
}

#[test]
fn hdr_highlights_keep_their_hue() {
    assert_eq!(to_rgb(hdr_to_srgb([10.0, 5.0, 0.0], 80.0)), Rgb::new(255, 188, 0));
}

#[test]
fn hdr_out_of_gamut_and_garbage_values_are_dropped() {
    assert_eq!(to_rgb(hdr_to_srgb([-0.3, 1.0, f32::NAN], 80.0)), Rgb::new(0, 255, 0));
    assert_eq!(to_rgb(hdr_to_srgb([f32::INFINITY, 0.0, 0.0], 80.0)), Rgb::BLACK);
    assert_eq!(to_rgb(hdr_to_srgb([1.0; 3], 0.0)), Rgb::new(255, 255, 255), "unknown white = 80 nits");
}

#[test]
fn srgb_encoding_matches_reference_points() {
    assert_eq!(srgb_encode(0.0), 0.0);
    assert!((srgb_encode(1.0) - 1.0).abs() < 1e-6);
    assert!((srgb_encode(0.002) - 0.02584).abs() < 1e-5, "linear toe");
}

#[test]
fn half_floats_decode() {
    assert_eq!(f16_to_f32(0x3C00), 1.0);
    assert_eq!(f16_to_f32(0x4000), 2.0);
    assert_eq!(f16_to_f32(0xC000), -2.0);
    assert_eq!(f16_to_f32(0x3800), 0.5);
    assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
    assert_eq!(f16_to_f32(0x7C00), f32::INFINITY);
    assert!(f16_to_f32(0x7E00).is_nan());
}

#[test]
fn black_threshold_switches_dark_leds_off() {
    let a = Adjust { black_threshold: 0.03, ..PLAIN };
    assert_eq!(adjust([0.02, 0.01, 0.0], &a), [0.0; 3]);
    assert!(approx(adjust([0.04, 0.01, 0.0], &a), [0.04, 0.01, 0.0]));
}

#[test]
fn saturation_moves_away_from_grey() {
    let c = [0.6, 0.4, 0.2];
    assert!(approx(adjust(c, &PLAIN), c));
    let more = adjust(c, &Adjust { saturation: 2.0, ..PLAIN });
    assert!(more[0] > c[0] && more[2] < c[2], "{more:?}");
    let grey = [0.5; 3];
    assert!(approx(adjust(grey, &Adjust { saturation: 2.0, ..PLAIN }), grey));
    let l = luma(c);
    let less = adjust(c, &Adjust { saturation: 0.5, ..PLAIN });
    assert!((less[0] - (l + (c[0] - l) * 0.5)).abs() < 1e-6);
}

#[test]
fn brightness_scales_the_result() {
    assert!(approx(adjust([1.0, 0.5, 0.0], &Adjust { brightness: 0.5, ..PLAIN }), [0.5, 0.25, 0.0]));
}

#[test]
fn adjust_reads_percentages_from_config() {
    let a =
        Adjust::from_config(&AmbilightConfig { brightness: 40, black_threshold: 5, ..AmbilightConfig::defaults(60) });
    assert_eq!(a, Adjust { saturation: 1.2, brightness: 0.4, black_threshold: 0.05 });
}

#[test]
fn to_rgb_clamps_and_rounds() {
    assert_eq!(to_rgb([1.5, -0.2, 0.5]), Rgb::new(255, 0, 128));
}

#[test]
fn mean_averages_the_rectangle() {
    let mut img = Image::filled(4, 2, [0.0; 3]);
    img.set(0, 0, [1.0, 0.0, 0.0]);
    img.set(1, 0, [0.0, 1.0, 0.0]);
    assert_eq!(img.mean(Rect { x0: 0, y0: 0, x1: 2, y1: 1 }), [0.5, 0.5, 0.0]);
    assert_eq!(img.mean(Rect { x0: 3, y0: 1, x1: 9, y1: 9 }), [0.0; 3], "clamped to the image");
    assert_eq!(img.mean(Rect { x0: 2, y0: 0, x1: 2, y1: 1 }), [0.0; 3], "empty = black");
}

#[test]
fn sample_square_stays_inside_the_area() {
    let area = Rect { x0: 0, y0: 10, x1: 160, y1: 80 };
    // depth 0.1 of the shorter side (70) = 7 px.
    let r = Image::sample_rect(area, [0.0, 0.0], 0.1);
    assert_eq!(r, Rect { x0: 0, y0: 10, x1: 7, y1: 17 });
    let r = Image::sample_rect(area, [1.0, 1.0], 0.1);
    assert_eq!(r, Rect { x0: 153, y0: 73, x1: 160, y1: 80 });
    let r = Image::sample_rect(area, [0.5, 0.5], 0.0);
    assert_eq!((r.width(), r.height()), (1, 1), "never smaller than one pixel");
}
