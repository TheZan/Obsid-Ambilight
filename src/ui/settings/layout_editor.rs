//! Layout editor: segments drawn over a screen schematic (front view), with
//! draggable ends, a segment list, the perimeter template and strip highlights.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::Backend;
use crate::ambilight::layout::{self, HighlightTarget, MAX_SEGMENTS, SegmentError};
use crate::app::commands::Command;
use crate::app::state::{Connection, Snapshot};
use crate::config::Segment;
use crate::i18n::{self, Strings};

/// Grid lines per screen side (5 %).
const GRID_STEPS: f64 = 20.0;
/// A dragged end snaps to a grid line closer than this.
pub const SNAP_DISTANCE: f64 = 0.015;
/// Highlights are repeated this often while shown; the worker drops them after 3 s.
const RESEND: Duration = Duration::from_secs(1);
/// Local edits win over the snapshot for this long.
const HOLD: Duration = Duration::from_millis(800);
/// Free LEDs taken by a new segment.
const NEW_SEGMENT_LEDS: usize = 10;
const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0xE5, 0x48, 0x4D),
    Color32::from_rgb(0x3D, 0xB2, 0x5A),
    Color32::from_rgb(0x3D, 0x7E, 0xE5),
    Color32::from_rgb(0xF2, 0xB3, 0x3D),
    Color32::from_rgb(0xA8, 0x5C, 0xE6),
    Color32::from_rgb(0x2C, 0xC5, 0xC9),
    Color32::from_rgb(0xE6, 0x6F, 0xB4),
    Color32::from_rgb(0x9A, 0xA0, 0xA6),
];

/// Clamps to the screen and snaps each coordinate to a 5 % grid line when close.
pub fn snap_point(p: [f64; 2]) -> [f64; 2] {
    p.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let g = (v * GRID_STEPS).round() / GRID_STEPS;
        if (v - g).abs() <= SNAP_DISTANCE { g } else { v }
    })
}

pub fn to_screen(rect: Rect, p: [f64; 2]) -> Pos2 {
    pos2(rect.left() + p[0] as f32 * rect.width(), rect.top() + p[1] as f32 * rect.height())
}

pub fn from_screen(rect: Rect, pos: Pos2) -> [f64; 2] {
    [
        f64::from(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0)),
        f64::from(((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0)),
    ]
}

/// Largest rectangle of the given width/height ratio centred in `avail`.
pub fn fit(avail: Rect, aspect: f32) -> Rect {
    let aspect = aspect.max(0.1);
    let (w, h) = if avail.width() / avail.height() > aspect {
        (avail.height() * aspect, avail.height())
    } else {
        (avail.width(), avail.width() / aspect)
    };
    Rect::from_center_size(avail.center(), vec2(w, h))
}

fn used(segments: &[Segment], led: usize) -> bool {
    segments.iter().any(|s| (s.lo()..=s.hi()).contains(&led))
}

/// First run of free LEDs (at most 10) for a new segment.
pub fn free_range(segments: &[Segment], led_count: usize) -> Option<(usize, usize)> {
    let start = (0..led_count).find(|&i| !used(segments, i))?;
    let end = (start..led_count).take_while(|&i| !used(segments, i)).take(NEW_SEGMENT_LEDS).last()?;
    Some((start, end))
}

pub fn unused_count(segments: &[Segment], led_count: usize) -> usize {
    (0..led_count).filter(|&i| !used(segments, i)).count()
}

fn error_text(s: &Strings, e: SegmentError) -> String {
    match e {
        SegmentError::OutOfRange { led_count } => {
            i18n::fill(s.seg_err_range, &[&led_count.saturating_sub(1).to_string()])
        }
        SegmentError::BadPoint => s.seg_err_point.to_owned(),
        SegmentError::Overlap(i) => i18n::fill(s.seg_err_overlap, &[&(i + 1).to_string()]),
    }
}

#[derive(Default)]
pub struct Edit {
    segments: Option<Vec<Segment>>,
    touched: Option<Instant>,
    selected: Option<usize>,
    find_led: bool,
    led: usize,
    show_segment: bool,
    perimeter: Option<[usize; 4]>,
    /// Last highlight sent and when.
    sent: Option<(HighlightTarget, Instant)>,
}

impl Edit {
    /// Ends a strip highlight started here (tab switch, nothing to show).
    pub fn leave(&mut self, backend: &Backend) {
        if self.sent.take().is_some() {
            backend.send(Command::StopHighlight);
        }
    }
}

fn point_editor(ui: &mut egui::Ui, p: &mut [f64; 2]) {
    ui.horizontal(|ui| {
        for c in p.iter_mut() {
            ui.add(egui::DragValue::new(c).range(0.0..=1.0).speed(0.005).fixed_decimals(2));
        }
    });
}

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, backend: &Backend, edit: &mut Edit, aspect: f32) {
    let led_count = snap.config.device.led_count;
    let saved = &snap.config.ambilight.segments;
    let held = edit.touched.is_some_and(|t| t.elapsed() < HOLD);
    // A local layout with errors is never sent, so it must not be overwritten either.
    let invalid = edit.segments.as_ref().is_some_and(|l| layout::errors(l, led_count).iter().any(Option::is_some));
    if edit.segments.is_none() || (!held && !invalid) {
        edit.segments = Some(saved.clone());
    }
    let mut segs = edit.segments.clone().unwrap_or_default();
    let before = segs.clone();
    if edit.selected.is_some_and(|i| i >= segs.len()) {
        edit.selected = None;
    }

    ui.heading(s.layout_heading);
    ui.weak(s.layout_hint);

    // Schematic with draggable ends.
    let width = ui.available_width().min(560.0);
    let (response, painter) = ui.allocate_painter(vec2(width, width * 0.6), Sense::hover());
    let screen = fit(response.rect.shrink(14.0), aspect);
    painter.rect_stroke(screen, 4.0, Stroke::new(1.5, ui.visuals().text_color()), StrokeKind::Middle);
    for (i, seg) in segs.iter_mut().enumerate() {
        let color = PALETTE[i % PALETTE.len()];
        let w = if edit.selected == Some(i) { 4.0 } else { 2.5 };
        let (a, b) = (to_screen(screen, seg.start), to_screen(screen, seg.end));
        painter.line_segment([a, b], Stroke::new(w, color));
        painter.arrow(a + (b - a) * 0.4, (b - a) * 0.2, Stroke::new(w, color));
        painter.text(a, Align2::RIGHT_BOTTOM, seg.from.to_string(), FontId::proportional(11.0), color);
        painter.text(b, Align2::LEFT_TOP, seg.to.to_string(), FontId::proportional(11.0), color);
        for (which, end) in [&mut seg.start, &mut seg.end].into_iter().enumerate() {
            let pos = to_screen(screen, *end);
            let id = ui.id().with(("segment-end", i, which));
            let r = ui.interact(Rect::from_center_size(pos, vec2(16.0, 16.0)), id, Sense::drag());
            if r.dragged()
                && let Some(p) = r.interact_pointer_pos()
            {
                *end = snap_point(from_screen(screen, p));
                edit.selected = Some(i);
            }
            painter.circle_filled(pos, if r.hovered() || r.dragged() { 6.0 } else { 4.0 }, color);
        }
    }

    // Segment list.
    ui.add_space(6.0);
    let errors = layout::errors(&segs, led_count);
    let mut remove = None;
    egui::Grid::new("segments").num_columns(7).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
        for h in ["", s.seg_from, s.seg_to, s.seg_start, s.seg_end, "", ""] {
            ui.label(h);
        }
        ui.end_row();
        for (i, seg) in segs.iter_mut().enumerate() {
            let label = egui::RichText::new(format!("#{}", i + 1)).color(PALETTE[i % PALETTE.len()]).strong();
            if ui.selectable_label(edit.selected == Some(i), label).clicked() {
                edit.selected = Some(i);
            }
            ui.add(egui::DragValue::new(&mut seg.from).range(0..=led_count - 1));
            ui.add(egui::DragValue::new(&mut seg.to).range(0..=led_count - 1));
            point_editor(ui, &mut seg.start);
            point_editor(ui, &mut seg.end);
            ui.horizontal(|ui| {
                if ui.small_button(s.seg_reverse).clicked() {
                    *seg = seg.reversed();
                }
                if ui.small_button(s.seg_delete).clicked() {
                    remove = Some(i);
                }
            });
            match errors[i] {
                Some(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, error_text(s, e));
                }
                None => {
                    ui.label("");
                }
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        segs.remove(i);
        edit.selected = None;
    }

    ui.horizontal(|ui| {
        let free = free_range(&segs, led_count).filter(|_| segs.len() < MAX_SEGMENTS);
        if ui.add_enabled(free.is_some(), egui::Button::new(s.seg_add)).clicked()
            && let Some((from, to)) = free
        {
            segs.push(Segment { from, to, start: [0.3, 0.5], end: [0.7, 0.5] });
            edit.selected = Some(segs.len() - 1);
        }
        if segs.is_empty() {
            ui.weak(s.no_segments);
        } else {
            ui.weak(i18n::fill(s.unused_leds, &[&unused_count(&segs, led_count).to_string()]));
        }
    });

    ui.horizontal(|ui| {
        let counts = edit.perimeter.get_or_insert_with(|| layout::default_counts(led_count));
        ui.label(s.perimeter_sides);
        for c in counts.iter_mut() {
            ui.add(egui::DragValue::new(c).range(0..=led_count));
        }
        let total: usize = counts.iter().sum();
        if ui.add_enabled(total > 0 && total <= led_count, egui::Button::new(s.perimeter_btn)).clicked() {
            segs = layout::perimeter(*counts);
            edit.selected = None;
        }
        if total > led_count {
            ui.colored_label(ui.visuals().error_fg_color, i18n::fill(s.perimeter_too_many, &[&led_count.to_string()]));
        }
    });

    if segs != before {
        edit.touched = Some(Instant::now());
        if layout::errors(&segs, led_count).iter().all(Option::is_none) {
            backend.send(Command::SetLayout { segments: segs.clone() });
        }
        edit.segments = Some(segs);
    }

    // Strip highlight.
    ui.separator();
    ui.horizontal(|ui| {
        ui.checkbox(&mut edit.find_led, s.find_led);
        ui.add_enabled(edit.find_led, egui::Slider::new(&mut edit.led, 0..=led_count - 1));
    });
    ui.checkbox(&mut edit.show_segment, s.show_on_strip);
    let target = if edit.find_led {
        Some(HighlightTarget::Led(edit.led.min(led_count - 1)))
    } else if edit.show_segment {
        edit.selected.filter(|i| *i < saved.len()).map(HighlightTarget::Segment)
    } else {
        None
    };
    let ready = snap.connection == Connection::Ready;
    if (edit.find_led || edit.show_segment) && !ready {
        ui.colored_label(ui.visuals().warn_fg_color, s.highlight_needs_strip);
    }
    match target.filter(|_| ready) {
        Some(t) => {
            if edit.sent.is_none_or(|(prev, at)| prev != t || at.elapsed() >= RESEND) {
                backend.send(Command::Highlight { target: t });
                edit.sent = Some((t, Instant::now()));
            }
            ui.ctx().request_repaint_after(RESEND);
        }
        None => edit.leave(backend),
    }
}
