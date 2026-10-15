//! The floating layout (Preferences, General, Layout): the toolbar, the browser, the feature
//! history and the command line float over a viewport that fills the window, instead of being
//! docked around it. Each is a card with a title strip you drag by, a button that minimizes it to
//! that strip, and (the browser) a corner to resize by. Where a card was left is kept in the
//! preferences as its offset from its default place, so a resized window keeps it sensible.
//!
//! The cards draw the same contents as the docked panels (`toolbar::toolbar_body`,
//! `browser::browser_body`, ...); only the frame around them differs.

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Order, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::SolveApp;
use crate::theme::Tokens;

/// Space between a card and the window's edge, and between stacked cards.
pub const MARGIN: f32 = 12.0;
/// The title strip's height.
pub const STRIP_H: f32 = 20.0;
/// Padding inside a card, around its strip and contents.
const PAD: f32 = 6.0;
/// Widest a minimized card is.
const MIN_W: f32 = 168.0;
/// The toolbar's contents are this tall (the docked toolbar's height).
pub const TOOLBAR_H: f32 = 92.0;
pub const TIMELINE_H: f32 = 40.0;
pub const PALETTE_H: f32 = 28.0;
/// How opaque a card is (the viewport shows faintly through it).
const OPACITY: f32 = 0.94;

/// The floating cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Toolbar,
    Browser,
    Timeline,
    Palette,
}

impl Which {
    fn resizable(self) -> bool {
        self == Which::Browser
    }

    fn index(self) -> u8 {
        self as u8
    }
}

/// Where one card was left.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Slot {
    /// From its default place (ui points).
    pub offset: [f32; 2],
    /// Only its title strip is shown.
    pub minimized: bool,
    /// Its contents' size, once the user has resized it (the browser).
    pub size: Option<[f32; 2]>,
}

/// Every card's place, kept in the preferences.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub toolbar: Slot,
    pub browser: Slot,
    pub timeline: Slot,
    pub palette: Slot,
}

impl Layout {
    pub fn slot(&self, which: Which) -> &Slot {
        match which {
            Which::Toolbar => &self.toolbar,
            Which::Browser => &self.browser,
            Which::Timeline => &self.timeline,
            Which::Palette => &self.palette,
        }
    }

    fn slot_mut(&mut self, which: Which) -> &mut Slot {
        match which {
            Which::Toolbar => &mut self.toolbar,
            Which::Browser => &mut self.browser,
            Which::Timeline => &mut self.timeline,
            Which::Palette => &mut self.palette,
        }
    }

    /// Every card back in its default place and size, and open.
    pub fn reset(&mut self) {
        *self = Layout::default();
    }

    /// Minimize every card, or open them all when all are minimized.
    pub fn toggle_all(&mut self) {
        let all = [Which::Toolbar, Which::Browser, Which::Timeline, Which::Palette].iter().all(|w| self.slot(*w).minimized);
        for w in [Which::Toolbar, Which::Browser, Which::Timeline, Which::Palette] {
            self.slot_mut(w).minimized = !all;
        }
    }
}

/// The card's outer size for contents of `size`.
fn outer_size(size: Vec2, minimized: bool) -> Vec2 {
    if minimized { vec2(size.x.min(MIN_W) + 2.0 * PAD, STRIP_H + 2.0 * PAD) } else { vec2(size.x + 2.0 * PAD, STRIP_H + size.y + 2.0 * PAD) }
}

/// Where a card of outer size `outer` goes by default in `avail` (the window below its title bar).
pub fn default_pos(which: Which, avail: Rect, outer: Vec2) -> Pos2 {
    let toolbar_h = outer_size(vec2(0.0, TOOLBAR_H), false).y;
    let timeline_h = outer_size(vec2(0.0, TIMELINE_H), false).y;
    match which {
        Which::Toolbar => pos2(avail.center().x - outer.x / 2.0, avail.top() + MARGIN),
        Which::Browser => pos2(avail.left() + MARGIN, avail.top() + MARGIN + toolbar_h + MARGIN),
        Which::Timeline => pos2(avail.center().x - outer.x / 2.0, avail.bottom() - MARGIN - outer.y.max(timeline_h)),
        Which::Palette => pos2(avail.center().x - outer.x / 2.0, avail.bottom() - MARGIN - timeline_h - 8.0 - outer.y),
    }
}

/// How far the view controls must move in from the viewport's top and bottom edges to clear the
/// cards: `(top, bottom)`. Only a card over the view cube (the top right corner) or over the
/// navigation bar (the bottom middle) matters, so a card dragged elsewhere frees the space again.
pub fn insets(cards: &[Rect], viewport: Rect, cube: f32) -> (f32, f32) {
    let cube_band = (viewport.right() - cube - 8.0, viewport.right());
    let nav_band = (viewport.center().x - 175.0, viewport.center().x + 175.0);
    let overlaps = |r: &Rect, band: (f32, f32)| r.right() > band.0 && r.left() < band.1;
    let (mut top, mut bottom) = (0.0_f32, 0.0_f32);
    for r in cards.iter().filter(|r| r.is_finite()) {
        if r.center().y < viewport.center().y && overlaps(r, cube_band) {
            top = top.max(r.bottom() - viewport.top() + 8.0);
        }
        if r.center().y >= viewport.center().y && overlaps(r, nav_band) {
            bottom = bottom.max(viewport.bottom() - r.top() + 8.0);
        }
    }
    (top, bottom)
}

/// `pos` moved so a card of outer size `outer` lies inside `avail` (a card larger than the
/// window sits at its top left).
pub fn clamp_pos(pos: Pos2, avail: Rect, outer: Vec2) -> Pos2 {
    let x = pos.x.clamp(avail.left(), (avail.right() - outer.x).max(avail.left()));
    let y = pos.y.clamp(avail.top(), (avail.bottom() - outer.y).max(avail.top()));
    pos2(x, y)
}

/// The width the toolbar and timeline take in `avail`.
pub fn wide(avail: Rect, max: f32) -> f32 {
    (avail.width() - 2.0 * (MARGIN + PAD)).clamp(160.0, max)
}

/// Lay out the floating cards (in place of the docked panels).
pub fn panels(app: &mut SolveApp, ui: &mut Ui) {
    app.viewport.float_cards.clear();
    let avail = ui.available_rect_before_wrap();
    card(app, ui, Which::Toolbar, crate::i18n::tr_label("TOOLS"), vec2(wide(avail, 1180.0), TOOLBAR_H), |app, ui| {
        crate::toolbar::toolbar_body(app, ui)
    });
    if app.ui.show_browser {
        crate::browser::browser_floating(app, ui, false);
    }
    if app.ui.show_timeline {
        card(app, ui, Which::Timeline, crate::i18n::tr_label("TIMELINE"), vec2(wide(avail, 1100.0), TIMELINE_H), |app, ui| {
            crate::timeline::timeline_body(app, ui)
        });
    }
    card(app, ui, Which::Palette, crate::i18n::tr_label("COMMAND"), vec2(wide(avail, 640.0), PALETTE_H), |app, ui| {
        crate::palette::palette_body(app, ui)
    });
}

/// Draw a floating card for `which`. `add` draws its contents into a `size` rectangle (a
/// resized card passes its own size instead).
pub fn card(app: &mut SolveApp, ui: &Ui, which: Which, title: &str, size: Vec2, add: impl FnOnce(&mut SolveApp, &mut Ui)) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get();
    let avail = ui.available_rect_before_wrap();
    let slot = app.preferences.floating.slot(which).clone();
    let size = match slot.size {
        Some([w, h]) if which.resizable() && w.is_finite() && h.is_finite() => {
            vec2(w.clamp(180.0, 700.0), h.clamp(120.0, (avail.height() - 2.0 * MARGIN).max(120.0)))
        }
        _ => size,
    };
    let minimized = slot.minimized;
    let outer = outer_size(size, minimized);
    let default = default_pos(which, avail, outer);
    let offset = vec2(slot.offset[0], slot.offset[1]);
    let offset = if offset.x.is_finite() && offset.y.is_finite() { offset } else { Vec2::ZERO };
    let pos = clamp_pos(default + offset, avail, outer);

    let mut moved_to: Option<Pos2> = None;
    let mut toggle = false;
    let mut resized_to: Option<Vec2> = None;
    let id = Id::new(("sc_float", which.index()));
    egui::Area::new(id).order(Order::Middle).fixed_pos(pos).constrain(false).show(&ctx, |ui| {
        let frame = Frame::new()
            .fill(t.panel.gamma_multiply(OPACITY))
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(90) })
            .inner_margin(Margin::same(PAD as i8));
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let inner_w = outer.x - 2.0 * PAD;
            let (strip, _) = ui.allocate_exact_size(vec2(inner_w, STRIP_H), Sense::hover());
            let (m, tg) = strip_ui(ui, &t, strip, which, title, minimized, pos);
            moved_to = m;
            toggle = tg;
            if !minimized {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(("sc_float_body", which.index())), |ui| {
                    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                    add(app, ui);
                });
                if which.resizable() {
                    let grip = Rect::from_min_size(rect.right_bottom() - vec2(16.0, 16.0), vec2(16.0, 16.0));
                    let resp = ui.interact(grip, id.with("resize"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeNwSe);
                    let c = if resp.hovered() || resp.dragged() { t.text } else { t.text_dim };
                    for k in [4.0, 8.0, 12.0] {
                        ui.painter().line_segment([grip.right_bottom() - vec2(k, 2.0), grip.right_bottom() - vec2(2.0, k)], Stroke::new(1.0, c));
                    }
                    // From where the corner was grabbed, so a press that arrives with a jump changes nothing.
                    let grab = id.with("resize_grab");
                    if resp.drag_started()
                        && let Some(p) = resp.interact_pointer_pos()
                    {
                        ui.data_mut(|d| d.insert_temp(grab, (p, size)));
                    }
                    if resp.dragged()
                        && let (Some(p), Some((p0, s0))) = (resp.interact_pointer_pos(), ui.data(|d| d.get_temp::<(Pos2, Vec2)>(grab)))
                    {
                        resized_to = Some(s0 + (p - p0));
                    }
                }
            }
        });
    });

    app.viewport.float_cards.push(Rect::from_min_size(pos, outer));
    let slot = app.preferences.floating.slot_mut(which);
    // Keep what clamping did to a card the window outgrew, so it does not snap back later.
    if let Some(target) = moved_to.or((pos != default + offset).then_some(pos)) {
        let o = clamp_pos(target, avail, outer) - default;
        slot.offset = [o.x, o.y];
    }
    if toggle {
        slot.minimized = !slot.minimized;
    }
    if let Some(s) = resized_to {
        slot.size = Some([s.x.clamp(180.0, 700.0), s.y.clamp(120.0, (avail.height() - 2.0 * MARGIN).max(120.0))]);
    }
}

/// The title strip: the grip dots and title (drag to move) and the minimize button. Returns where
/// a drag of the card (at `pos`) has put it, and whether the button was clicked. The card follows
/// the pointer from where it was grabbed, so a press that arrives with a jump (a tap on a touch
/// screen) moves nothing.
fn strip_ui(ui: &mut Ui, t: &Tokens, strip: Rect, which: Which, title: &str, minimized: bool, pos: Pos2) -> (Option<Pos2>, bool) {
    let painter = ui.painter().clone();
    let id = Id::new(("sc_float_strip", which.index()));
    let button = Rect::from_center_size(pos2(strip.right() - 11.0, strip.center().y), vec2(20.0, 18.0));
    // The button is not part of the handle, so pressing it never starts a drag.
    let handle = Rect::from_min_max(strip.min, pos2(button.left() - 2.0, strip.bottom()));
    let drag = ui.interact(handle, id, Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
    let hit = ui.interact(button, id.with("minimize"), Sense::click());
    let grab = id.with("grab");
    if drag.drag_started()
        && let Some(p) = drag.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(grab, p - pos));
    }
    let moved = if drag.dragged()
        && let (Some(p), Some(offset)) = (drag.interact_pointer_pos(), ui.data(|d| d.get_temp::<Vec2>(grab)))
    {
        Some(p - offset)
    } else {
        None
    };
    // The grip: two columns of dots, then the title.
    let dot = if drag.hovered() || drag.dragged() { t.text } else { t.text_dim };
    for col in 0..2 {
        for row in 0..3 {
            painter.circle_filled(pos2(strip.left() + 7.0 + col as f32 * 4.0, strip.center().y - 4.0 + row as f32 * 4.0), 0.9, dot);
        }
    }
    painter.text(pos2(strip.left() + 20.0, strip.center().y), Align2::LEFT_CENTER, title, FontId::proportional(10.5), t.text_dim);
    if hit.hovered() {
        painter.rect_filled(button, 4.0, t.hover);
    }
    let c = if hit.hovered() { t.text } else { t.text_dim };
    let m = button.center();
    if minimized {
        // Open: a small square.
        painter.rect_stroke(Rect::from_center_size(m, vec2(8.0, 7.0)), 1.5, Stroke::new(1.3, c), egui::StrokeKind::Inside);
    } else {
        // Minimize: a bar.
        painter.line_segment([m + vec2(-4.0, 2.0), m + vec2(4.0, 2.0)], Stroke::new(1.5, c));
    }
    let tip = if minimized { crate::i18n::tr_label("Show") } else { crate::i18n::tr_label("Minimize") };
    let clicked = hit.on_hover_text(tip).clicked();
    (moved, clicked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Harness;
    use serde_json::json;

    #[test]
    fn the_floating_layout_floats_every_card_over_a_full_window_viewport() {
        let mut h = Harness::new();
        h.call("ui.resize", json!({"width": 1400.0, "height": 900.0}));
        h.settle();
        let docked = h.app.viewport.rect.expect("a viewport");
        assert!(h.app.viewport.float_cards.is_empty(), "docked panels are not cards");

        h.app.preferences.floating_layout = true;
        h.settle();
        let cards = h.app.viewport.float_cards.clone();
        assert_eq!(cards.len(), 4, "toolbar, browser, timeline and command line: {cards:?}");
        let full = h.app.viewport.rect.expect("a viewport");
        assert!(full.width() > docked.width() + 200.0 && full.height() > docked.height() + 100.0, "{docked:?} -> {full:?}");
        for c in &cards {
            assert!(full.contains_rect(*c), "a card must lie inside the window: {c:?} in {full:?}");
        }
        let layout = h.app.viewport.layout.clone().expect("a layout");
        for c in &cards {
            assert!(!layout.navigation.intersect(*c).is_positive(), "navigation bar under a card: {:?} {c:?}", layout.navigation);
            assert!(!layout.axis_view.intersect(*c).is_positive(), "view cube under a card: {:?} {c:?}", layout.axis_view);
        }

        // Minimize all: only the strips are left, and the view controls come back down.
        h.app.preferences.floating.toggle_all();
        h.settle();
        assert!(h.app.viewport.float_cards.iter().all(|c| c.height() <= STRIP_H + 2.0 * PAD + 0.5), "{:?}", h.app.viewport.float_cards);
        h.app.preferences.floating.toggle_all();
        h.settle();
        assert!(h.app.viewport.float_cards.iter().all(|c| c.height() > 40.0));

        // Back to docked: the cards go, and the viewport is the old size again.
        h.app.preferences.floating_layout = false;
        h.settle();
        assert!(h.app.viewport.float_cards.is_empty());
        assert_eq!(h.app.viewport.rect, Some(docked));
    }

    #[test]
    fn old_preferences_open_docked_and_a_saved_layout_comes_back() {
        let old: crate::prefs::Prefs = serde_json::from_value(json!({"theme": "light"})).unwrap();
        assert!(!old.floating_layout);
        assert_eq!(old.floating, Layout::default());
        let saved: crate::prefs::Prefs =
            serde_json::from_value(json!({"floating_layout": true, "floating": {"browser": {"minimized": true, "offset": [10.0, 20.0]}}})).unwrap();
        assert!(saved.floating_layout && saved.floating.browser.minimized);
        assert_eq!(saved.floating.browser.offset, [10.0, 20.0]);
        assert!(!saved.floating.toolbar.minimized, "an unmentioned card stays open");
    }

    fn window() -> Rect {
        Rect::from_min_size(pos2(0.0, 34.0), vec2(1440.0, 866.0))
    }

    #[test]
    fn the_toolbar_is_centred_at_the_top_and_the_browser_sits_below_its_left_end() {
        let tb = vec2(1000.0, 130.0);
        let p = default_pos(Which::Toolbar, window(), tb);
        assert_eq!(p.x, 220.0);
        assert_eq!(p.y, 34.0 + MARGIN);
        let b = default_pos(Which::Browser, window(), vec2(262.0, 500.0));
        assert_eq!(b.x, MARGIN);
        assert!(b.y > p.y + tb.y, "the browser must start below the toolbar card");
    }

    #[test]
    fn the_command_line_sits_above_the_timeline() {
        let tl = outer_size(vec2(1000.0, TIMELINE_H), false);
        let pl = outer_size(vec2(600.0, PALETTE_H), false);
        let t = default_pos(Which::Timeline, window(), tl);
        let p = default_pos(Which::Palette, window(), pl);
        assert!(p.y + pl.y < t.y, "palette {p:?} must end above the timeline {t:?}");
        assert!(t.y + tl.y <= window().bottom() - MARGIN + 0.01);
    }

    #[test]
    fn a_dragged_card_stays_inside_the_window() {
        let outer = vec2(300.0, 200.0);
        assert_eq!(clamp_pos(pos2(-50.0, -50.0), window(), outer), pos2(0.0, 34.0));
        assert_eq!(clamp_pos(pos2(5000.0, 5000.0), window(), outer), pos2(1140.0, 700.0));
        // A card wider than the window sits at the left edge instead of panicking.
        assert_eq!(clamp_pos(pos2(10.0, 40.0), window(), vec2(9000.0, 100.0)).x, 0.0);
    }

    #[test]
    fn view_controls_clear_only_the_cards_over_them() {
        let vp = Rect::from_min_size(pos2(0.0, 34.0), vec2(1440.0, 866.0));
        let toolbar = Rect::from_min_size(pos2(130.0, 46.0), vec2(1290.0, 130.0));
        let timeline = Rect::from_min_size(pos2(170.0, 835.0), vec2(1100.0, 58.0));
        let browser = Rect::from_min_size(pos2(12.0, 200.0), vec2(262.0, 500.0));
        let (top, bottom) = insets(&[toolbar, timeline, browser], vp, 100.0);
        assert!(top >= toolbar.bottom() - vp.top(), "the cube must sit below the toolbar: {top}");
        assert!(bottom >= vp.bottom() - timeline.top(), "the navigation bar must sit above the timeline: {bottom}");
        // A toolbar dragged to the left of the cube, and a timeline dragged away from the middle.
        let moved_toolbar = Rect::from_min_size(pos2(12.0, 46.0), vec2(900.0, 130.0));
        let moved_timeline = Rect::from_min_size(pos2(12.0, 835.0), vec2(300.0, 58.0));
        assert_eq!(insets(&[moved_toolbar, moved_timeline, browser], vp, 100.0), (0.0, 0.0));
    }

    #[test]
    fn minimizing_leaves_only_the_strip() {
        let open = outer_size(vec2(250.0, 400.0), false);
        let shut = outer_size(vec2(250.0, 400.0), true);
        assert_eq!(shut.y, STRIP_H + 2.0 * PAD);
        assert!(shut.x <= MIN_W + 2.0 * PAD && shut.x < open.x);
    }

    #[test]
    fn toggle_all_minimizes_then_opens() {
        let mut l = Layout::default();
        l.toggle_all();
        assert!(l.toolbar.minimized && l.browser.minimized && l.timeline.minimized && l.palette.minimized);
        l.toggle_all();
        assert!(!l.toolbar.minimized && !l.browser.minimized);
        l.browser.minimized = true;
        l.toggle_all();
        assert!(l.toolbar.minimized, "one open card means minimize all");
    }

    #[test]
    fn the_layout_survives_a_round_trip_and_an_old_file_without_it() {
        let mut l = Layout::default();
        l.browser.offset = [30.0, -12.0];
        l.browser.size = Some([300.0, 420.0]);
        l.timeline.minimized = true;
        let back: Layout = serde_json::from_value(serde_json::to_value(&l).unwrap()).unwrap();
        assert_eq!(back, l);
        let empty: Layout = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(empty, Layout::default());
        l.reset();
        assert_eq!(l, Layout::default());
    }
}
