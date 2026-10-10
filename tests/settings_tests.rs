use std::net::Ipv4Addr;
use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{Connection, DeviceInfo};
use obsid_ambilight::config::Config;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::settings::diagnostics::summary;

#[test]
fn summary_lists_key_facts_without_secrets() {
    let mut s = Controller::new(
        Config::default(),
        vec!["w".into()],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(true);
    s.connection = Connection::Ready;
    s.device = Some(DeviceInfo {
        id: "dcda0cd778bc".into(),
        name: String::new(),
        ip: Ipv4Addr::new(192, 168, 1, 85),
        fw_ver: Some("36".into()),
    });
    s.errors = vec!["timeout".into()];
    let text = summary(&s);
    for needle in
        ["Obsid Ambilight", env!("CARGO_PKG_VERSION"), "192.168.1.85", "dcda0cd778bc", "fw 36", "Ready", "timeout", "w"]
    {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
    assert!(!text.to_lowercase().contains("token"));
}

#[test]
fn hex_commit_applies_only_real_changes() {
    use obsid_ambilight::protocol::Rgb;
    use obsid_ambilight::ui::settings::lighting::{HexCommit, hex_commit};
    let current = Rgb::new(0xF5, 0xD4, 0xA1);
    assert_eq!(hex_commit("#123456", current), HexCommit::Apply(Rgb::new(0x12, 0x34, 0x56)));
    assert_eq!(hex_commit(" 123456 ", current), HexCommit::Apply(Rgb::new(0x12, 0x34, 0x56)));
    assert_eq!(
        hex_commit("#f5d4a1", current),
        HexCommit::Unchanged,
        "focus loss on the same colour must not power the strip on"
    );
    assert_eq!(hex_commit("#000000", current), HexCommit::Black);
    assert_eq!(hex_commit("zz", current), HexCommit::Invalid);
}

#[test]
fn summary_reports_mode_stream_and_capture() {
    use obsid_ambilight::app::state::CaptureStatus;
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(true);
    let text = summary(&s);
    assert!(text.contains("mode: Static"), "{text}");
    assert!(text.contains("capture: not running"), "{text}");
    s.stream.streaming = true;
    s.stream.capture = Some(CaptureStatus {
        monitor: r"\\.\DISPLAY1".into(),
        adapter: "GTX".into(),
        width: 2560,
        height: 1440,
        hdr: true,
        sdr_white_nits: 240.0,
        fps: 25.0,
        active: true,
        ..CaptureStatus::default()
    });
    let text = summary(&s);
    for needle in [r"\\.\DISPLAY1", "GTX", "2560x1440", "hdr true", "240", "25.0 fps", "stream: true"] {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
}

#[test]
fn monitor_aspect_follows_the_setting() {
    use obsid_ambilight::capture::outputs::MonitorInfo;
    use obsid_ambilight::ui::settings::ambilight::aspect_of;
    let monitors = [
        MonitorInfo { name: r"\\.\DISPLAY1".into(), adapter: "A".into(), width: 2560, height: 1440, primary: true },
        MonitorInfo { name: r"\\.\DISPLAY2".into(), adapter: "B".into(), width: 1280, height: 1024, primary: false },
    ];
    assert_eq!(aspect_of(&monitors, ""), 2560.0 / 1440.0);
    assert_eq!(aspect_of(&monitors, r"\\.\DISPLAY2"), 1280.0 / 1024.0);
    assert_eq!(aspect_of(&monitors, "gone"), 16.0 / 9.0);
    assert_eq!(aspect_of(&[], ""), 16.0 / 9.0);
}

#[test]
fn ambilight_tab_is_a_command_line_tab() {
    use obsid_ambilight::app::state::Tab;
    assert_eq!(Tab::from_arg("ambilight"), Some(Tab::Ambilight));
    assert_eq!(Tab::from_index(Tab::Ambilight.index()), Tab::Ambilight);
}

mod layout_editor {
    use eframe::egui::{Rect, pos2, vec2};
    use obsid_ambilight::config::Segment;
    use obsid_ambilight::ui::settings::layout_editor::{
        connectors, fit, free_range, from_screen, label_pos, snap_end, snap_point, to_screen, unused_count,
    };

    fn seg(from: usize, to: usize) -> Segment {
        Segment { from, to, start: [0.0, 0.0], end: [1.0, 0.0] }
    }

    #[test]
    fn ends_snap_to_the_5_percent_grid_when_close() {
        assert_eq!(snap_point([0.012, 0.5]), [0.0, 0.5]);
        assert_eq!(snap_point([0.03, 0.27]), [0.03, 0.27], "too far from a grid line");
        assert_eq!(snap_point([0.991, 1.2]), [1.0, 1.0], "clamped, then snapped");
    }

    #[test]
    fn screen_mapping_round_trips() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 225.0));
        assert_eq!(to_screen(r, [0.5, 1.0]), pos2(210.0, 245.0));
        assert_eq!(from_screen(r, pos2(210.0, 245.0)), [0.5, 1.0]);
        assert_eq!(from_screen(r, pos2(-50.0, 999.0)), [0.0, 1.0], "outside points are clamped");
    }

    #[test]
    fn schematic_keeps_the_monitor_aspect() {
        let r = fit(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)), 16.0 / 9.0);
        assert_eq!((r.width(), r.height()), (400.0, 225.0));
        assert_eq!(r.center(), pos2(200.0, 150.0));
    }

    #[test]
    fn new_segments_take_the_first_free_leds() {
        assert_eq!(free_range(&[], 60), Some((0, 9)));
        assert_eq!(free_range(&[seg(0, 9), seg(20, 15)], 60), Some((10, 14)));
        assert_eq!(free_range(&[seg(0, 59)], 60), None);
        assert_eq!(unused_count(&[seg(0, 9), seg(20, 15)], 60), 44);
    }

    #[test]
    fn a_dragged_end_sticks_to_a_nearby_end() {
        let others = [[0.3, 0.98], [0.9, 0.4]];
        assert_eq!(snap_end([0.31, 0.96], &others), [0.3, 0.98], "joins the closer end");
        assert_eq!(snap_end([0.5, 0.5], &others), [0.5, 0.5], "nothing near: grid rules apply");
        assert_eq!(snap_end([0.512, 0.5], &others), [0.5, 0.5]);
    }

    #[test]
    fn connectors_follow_the_strip_between_segments() {
        let segs = [
            Segment { from: 33, to: 42, start: [0.98, 0.98], end: [0.3, 0.98] },
            Segment { from: 43, to: 47, start: [0.28, 0.92], end: [0.28, 0.42] },
            Segment { from: 59, to: 48, start: [0.9, 0.4], end: [0.28, 0.4] },
            Segment { from: 0, to: 9, start: [0.02, 0.98], end: [0.02, 0.02] },
        ];
        let c = connectors(&segs);
        assert!(c.contains(&([0.3, 0.98], [0.28, 0.92])), "42 -> 43: {c:?}");
        assert!(c.contains(&([0.28, 0.42], [0.28, 0.4])), "47 -> 48 at the end of a reversed segment: {c:?}");
        assert_eq!(c.len(), 2, "no link from 9: LED 10 is in no segment");
    }

    #[test]
    fn labels_stay_inside_and_apart_at_a_corner() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 225.0));
        let corner = pos2(0.0, 0.0);
        let down = label_pos(screen, corner, pos2(0.0, 225.0));
        let right = label_pos(screen, corner, pos2(400.0, 0.0));
        for p in [down, right] {
            assert!(screen.shrink(4.0).contains(p), "{p:?}");
        }
        assert!((down - right).length() > 10.0, "{down:?} vs {right:?}");
    }
}
