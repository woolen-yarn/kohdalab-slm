//! Fixed logical-pixel geometry in the style of KohdaLab TRKR
//! (`trkr-desktop/src/precision.rs`): absolute rectangles inside cards, the
//! same control heights (26 px), card radius (10), borders and column widths.
//! A local copy rather than a cross-app dependency, as the GUI skill asks.

use crate::theme::{BORDER, BUTTON, INPUT, MUTED, PANEL, PRIMARY, TEXT};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w.max(0.), h.max(0.)))
}

/// Three columns as in TRKR: left 384, center ≥ 850, right 300.
pub struct Columns {
    pub left: Rect,
    pub left_toggle: Rect,
    pub measurement: Rect,
    pub toolbar: Rect,
    pub list: Rect,
    pub toggle: Rect,
    pub log: Rect,
    pub table: Rect,
}

impl Columns {
    pub const WIDTH: f32 = 850.;
    pub const LEFT_WIDTH: f32 = 384.;
    pub const RIGHT_WIDTH: f32 = 300.;
    pub const TOP: f32 = 235.;

    pub fn minimum_width(right_collapsed: bool, left_collapsed: bool) -> f32 {
        (if left_collapsed {
            40.
        } else {
            Self::LEFT_WIDTH
        }) + Self::WIDTH
            + 32.
            + if right_collapsed {
                27.
            } else {
                Self::RIGHT_WIDTH
            }
    }

    pub fn new(viewport: Rect, right_collapsed: bool, left_collapsed: bool) -> Self {
        let (w, h) = (viewport.width(), viewport.height());
        let left = if left_collapsed {
            40.
        } else {
            Self::LEFT_WIDTH
        };
        let right = if right_collapsed {
            27.
        } else {
            Self::RIGHT_WIDTH
        };
        let center_width = (w - left - right - 32.).max(Self::WIDTH);
        let c = rect(left + 16., 8., center_width, h - 16.);
        let measurement = rect(c.min.x + 9., 17., c.width() - 18., Self::TOP);
        let list_y = measurement.max.y + 40.;
        let right_x = c.max.x + 8.;
        let o = viewport.min.to_vec2();
        let mov = |r: Rect| r.translate(o);
        Self {
            left: mov(rect(8., 8., left, h - 16.)),
            left_toggle: mov(rect(left - 24., 8., 24., 28.)),
            measurement: mov(measurement),
            toolbar: mov(rect(
                c.min.x + 9.,
                measurement.max.y + 8.,
                c.width() - 18.,
                24.,
            )),
            list: mov(rect(
                c.min.x + 9.,
                list_y,
                c.width() - 18.,
                h - 17. - list_y,
            )),
            toggle: mov(rect(
                right_x + if right_collapsed { 1. } else { 0. },
                8.,
                24.,
                28.,
            )),
            log: mov(rect(right_x + 37., 17., right - 46., Self::TOP)),
            table: mov(rect(
                right_x + 37.,
                17. + Self::TOP + 8.,
                right - 46.,
                h - 34. - Self::TOP - 8.,
            )),
        }
    }
}

/// Startup window fit for the monitor the window opened on (sizes in
/// logical points, so DPI scaling and RDP resolutions are already applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowFit {
    /// Requested size does not fit: maximize so the OS fits the work area.
    pub maximize: bool,
    pub collapse_right: bool,
    pub collapse_left: bool,
    /// Usable area after reserving taskbar and title bar.
    pub usable: Vec2,
    /// Minimum inner size, never larger than the usable area.
    pub min_inner: Vec2,
}

/// Room kept for the taskbar and the window frame/title bar [points].
pub const SCREEN_RESERVE: Vec2 = Vec2::new(16., 96.);
pub const MIN_HEIGHT: f32 = 700.;

pub fn fit_window(monitor: Vec2, requested: Vec2) -> WindowFit {
    let usable = (monitor - SCREEN_RESERVE).max(Vec2::new(320., 240.));
    let maximize = requested.x > usable.x || requested.y > usable.y;
    // Keep the TRKR column layout: give up the log column first, then the
    // device column, only when the screen is narrower than the layout.
    let collapse_right = usable.x < Columns::minimum_width(false, false);
    let collapse_left = collapse_right && usable.x < Columns::minimum_width(true, false);
    let min_width = Columns::minimum_width(collapse_right, collapse_left);
    WindowFit {
        maximize,
        collapse_right,
        collapse_left,
        usable,
        min_inner: Vec2::new(min_width.min(usable.x), MIN_HEIGHT.min(usable.y)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Primary,
}

pub struct Form<'a> {
    pub ui: &'a mut egui::Ui,
    pub origin: Pos2,
    pub enabled: bool,
}

impl<'a> Form<'a> {
    pub fn new(ui: &'a mut egui::Ui, origin: Pos2) -> Self {
        Self {
            ui,
            origin,
            enabled: true,
        }
    }
    pub fn at(&self, local: Rect) -> Rect {
        local.translate(self.origin.to_vec2())
    }

    pub fn group(&mut self, local: Rect, title: &str) {
        let r = self.at(local);
        self.ui.painter().rect(
            r,
            10.,
            PANEL,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        if !title.is_empty() {
            self.ui.painter().text(
                r.min + Vec2::new(10., 6.),
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::proportional(12.),
                TEXT,
            );
        }
    }

    /// Painted text, vertically centered and clipped to its rectangle.
    pub fn value(&mut self, local: Rect, text: impl Into<String>, color: Color32) {
        let r = self.at(local);
        let galley =
            self.ui
                .painter()
                .layout_no_wrap(text.into(), egui::FontId::proportional(12.), color);
        let pos = Pos2::new(r.min.x, r.center().y - galley.size().y * 0.5);
        self.ui
            .painter()
            .with_clip_rect(r.intersect(self.ui.clip_rect()))
            .galley(pos, galley, color);
    }
    pub fn label(&mut self, local: Rect, text: impl Into<String>) {
        self.value(local, text, TEXT);
    }
    pub fn muted(&mut self, local: Rect, text: impl Into<String>) {
        self.value(local, text, MUTED);
    }

    /// Accessible text with a full-value hover (long names, reasons).
    pub fn note(&mut self, local: Rect, text: &str, color: Color32) -> egui::Response {
        let r = self.at(local);
        self.ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(r)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(r));
                    ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate())
                        .on_hover_text(text)
                },
            )
            .inner
    }

    fn scoped<R>(&mut self, r: Rect, enabled: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let all = self.enabled;
        self.ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(r)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    if !enabled || !all {
                        ui.disable();
                    }
                    add(ui)
                },
            )
            .inner
    }

    pub fn button_with(
        &mut self,
        local: Rect,
        text: &str,
        enabled: bool,
        tone: Tone,
        hover: Option<&str>,
    ) -> bool {
        let r = self.at(local);
        let active = enabled && self.enabled;
        let (fill, color) = match (tone, active) {
            (Tone::Primary, true) => (PRIMARY, Color32::WHITE),
            _ => (BUTTON, TEXT),
        };
        self.scoped(r, enabled, |ui| {
            let response = ui.put(
                r,
                egui::Button::new(egui::RichText::new(text).color(color))
                    .fill(fill)
                    .min_size(r.size())
                    .corner_radius(6)
                    .truncate(),
            );
            let response = match hover {
                Some(h) => response.on_hover_text(h).on_disabled_hover_text(h),
                None => response,
            };
            response.clicked()
        })
    }
    pub fn button(&mut self, local: Rect, text: &str, enabled: bool) -> bool {
        self.button_with(local, text, enabled, Tone::Neutral, None)
    }

    pub fn text(&mut self, id: &str, local: Rect, value: &mut String) -> egui::Response {
        let r = self.at(local);
        self.scoped(r, true, |ui| {
            ui.put(
                r,
                egui::TextEdit::singleline(value)
                    .id(egui::Id::new(id))
                    .margin(Vec2::new(4., 2.))
                    .min_size(r.size())
                    .vertical_align(egui::Align::Center)
                    .desired_width(r.width()),
            )
        })
    }

    /// TRKR `path`: closed `…/name` summary; click to edit the full value.
    pub fn path(&mut self, id: &str, local: Rect, value: &mut String) {
        let r = self.at(local);
        let edit_id = egui::Id::new((id, "path_edit"));
        let editing = self.ui.ctx().data(|d| d.get_temp::<bool>(edit_id));
        self.scoped(r, true, |ui| {
            if let Some(opening) = editing {
                let response = ui.put(
                    r,
                    egui::TextEdit::singleline(value)
                        .id(egui::Id::new(id))
                        .margin(Vec2::new(4., 2.))
                        .min_size(r.size())
                        .vertical_align(egui::Align::Center)
                        .desired_width(r.width()),
                );
                if opening {
                    response.request_focus();
                    ui.ctx().data_mut(|d| d.insert_temp(edit_id, false));
                }
                let close = response.lost_focus()
                    || ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)
                    });
                if close {
                    response.surrender_focus();
                    ui.ctx().data_mut(|d| d.remove::<bool>(edit_id));
                }
                response.on_hover_text(value.as_str());
            } else if input_like(ui, r, &summary(value))
                .on_hover_text(format!("{value}\nClick to edit the full path."))
                .clicked()
            {
                ui.ctx().data_mut(|d| d.insert_temp(edit_id, true));
                ui.ctx().request_repaint();
            }
        });
    }

    /// Closed summary `…/name` with the full value on hover (TRKR `path`).
    pub fn path_summary(&mut self, local: Rect, value: &str) -> bool {
        let r = self.at(local);
        let summary = summary(value);
        self.scoped(r, true, |ui| {
            input_like(ui, r, &summary).on_hover_text(value).clicked()
        })
    }

    pub fn check(&mut self, local: Rect, value: &mut bool, label: &str) -> bool {
        let r = self.at(local);
        self.scoped(r, true, |ui| {
            ui.put(r, egui::Checkbox::new(value, label)).changed()
        })
    }

    /// Numeric field: compact display, full precision kept while editing.
    pub fn number(&mut self, id: &str, local: Rect, value: &mut f64, enabled: bool) -> bool {
        let r = self.at(local);
        let id = egui::Id::new(id);
        self.scoped(r, enabled, |ui| {
            let focused = ui.memory(|m| m.has_focus(id));
            let mut text = if focused {
                ui.data(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| value.to_string())
            } else {
                let full = slm_core::units::format_number(*value);
                if full.len() > 8 { format!("{:.4e}", *value) } else { full }
            };
            let response = ui.put(
                r,
                egui::TextEdit::singleline(&mut text)
                    .id(id)
                    .margin(Vec2::new(4., 2.))
                    .min_size(r.size())
                    .vertical_align(egui::Align::Center)
                    .desired_width(r.width()),
            );
            let parsed = text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
            if response.has_focus() && parsed.is_none() {
                ui.painter().rect_stroke(
                    r,
                    6.,
                    egui::Stroke::new(1_f32, crate::theme::ERROR),
                    egui::StrokeKind::Inside,
                );
            }
            let response = if parsed.is_none() {
                response.on_hover_text("Enter a finite number. The last valid value is retained; empty text is not applied.")
            } else { response.on_hover_text(format!("Full value: {value}")) };
            // Keep full precision on focus; compact display is never parsed into the model without an edit.
            if response.gained_focus() && !response.changed() { text = value.to_string(); }
            ui.data_mut(|d| d.insert_temp(id, text));
            match parsed {
                Some(v) if response.changed() && v != *value => {
                    *value = v;
                    true
                }
                _ => false,
            }
        })
    }

    pub fn integer<T>(&mut self, id: &str, local: Rect, value: &mut T, min: f64, max: f64) -> bool
    where
        T: Copy + Into<f64> + TryFrom<i64>,
    {
        let mut v: f64 = (*value).into();
        if self.number(id, local, &mut v, true) {
            let rounded = v.round().clamp(min, max) as i64;
            if let Ok(n) = T::try_from(rounded) {
                *value = n;
                return true;
            }
        }
        false
    }

    pub fn combo<T: PartialEq + Clone>(
        &mut self,
        id: &str,
        local: Rect,
        value: &mut T,
        options: &[(T, String, bool)],
    ) -> bool {
        self.combo_closed(id, local, value, options, None)
    }

    /// Combo whose closed field shows `closed` (short) while the dropdown
    /// lists the full option labels.
    pub fn combo_closed<T: PartialEq + Clone>(
        &mut self,
        id: &str,
        local: Rect,
        value: &mut T,
        options: &[(T, String, bool)],
        closed: Option<String>,
    ) -> bool {
        let r = self.at(local);
        let before = value.clone();
        self.scoped(r, true, |ui| {
            ui.spacing_mut().combo_width = r.width();
            ui.spacing_mut().interact_size.y = r.height();
            let label = closed.clone().unwrap_or_else(|| {
                options
                    .iter()
                    .find(|(v, _, _)| v == value)
                    .map_or_else(|| "—".to_string(), |(_, s, _)| s.clone())
            });
            egui::ComboBox::from_id_salt(id)
                .width(r.width())
                .truncate()
                .selected_text(label)
                .show_ui(ui, |ui| {
                    ui.set_min_width(r.width());
                    for (v, label, enabled) in options {
                        ui.add_enabled_ui(*enabled, |ui| {
                            ui.selectable_value(value, v.clone(), label);
                        });
                    }
                });
        });
        *value != before
    }

    pub fn segments<T: PartialEq + Clone>(
        &mut self,
        local: Rect,
        value: &mut T,
        options: &[(T, &str)],
    ) -> bool {
        let r = self.at(local);
        let n = options.len().max(1) as f32;
        let width = (r.width() - 3. * (n - 1.)) / n;
        let mut changed = false;
        for (i, (v, label)) in options.iter().enumerate() {
            let segment = Rect::from_min_size(
                r.min + Vec2::new(i as f32 * (width + 3.), 0.),
                Vec2::new(width, r.height()),
            );
            let clicked = self.scoped(segment, true, |ui| {
                ui.put(
                    segment,
                    egui::Button::new(*label)
                        .selected(value == v)
                        .min_size(segment.size())
                        .corner_radius(6),
                )
                .clicked()
            });
            if clicked && value != v {
                *value = v.clone();
                changed = true;
            }
        }
        changed
    }

    /// TRKR disclosure button: triangle plus caption.
    pub fn toggle(&mut self, local: Rect, expanded: &mut bool, label: &str) {
        let r = self.at(local);
        let clicked = self.ui.put(
            r,
            egui::Button::new(format!("     {label}"))
                .min_size(r.size())
                .corner_radius(6),
        );
        if clicked.clicked() {
            *expanded = !*expanded;
        }
        let c = r.min + Vec2::new(11., r.height() * 0.5);
        let points = if *expanded {
            vec![
                c + Vec2::new(-3., -2.),
                c + Vec2::new(3., -2.),
                c + Vec2::new(0., 2.),
            ]
        } else {
            vec![
                c + Vec2::new(-2., -3.),
                c + Vec2::new(-2., 3.),
                c + Vec2::new(2., 0.),
            ]
        };
        self.ui.painter().add(egui::Shape::convex_polygon(
            points,
            Color32::LIGHT_GRAY,
            egui::Stroke::NONE,
        ));
    }

    pub fn dot(&mut self, local_center: Pos2, color: Color32) {
        self.ui
            .painter()
            .circle_filled(local_center + self.origin.to_vec2(), 3., color);
    }
}

/// Read-only value drawn like a text field (left-aligned, input fill,
/// border), clickable. Used for path summaries so they align with editable
/// text fields instead of looking like centered buttons.
fn input_like(ui: &mut egui::Ui, r: Rect, text: &str) -> egui::Response {
    let response = ui.interact(r, ui.id().with(("input-like", text)), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        r,
        6.,
        INPUT,
        egui::Stroke::new(
            1_f32,
            if response.hovered() {
                visuals.bg_stroke.color
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let color = if ui.is_enabled() { TEXT } else { MUTED };
    let inner = r.shrink2(Vec2::new(6., 0.));
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        egui::FontId::proportional(12.),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width());
    let galley = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(r).galley(
        Pos2::new(inner.min.x, r.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn summary(value: &str) -> String {
    let trimmed = value.trim_end_matches(['/', '\\']);
    match trimmed.rsplit_once(['/', '\\']) {
        Some((_, tail)) if !tail.is_empty() => format!("…/{tail}"),
        _ => value.to_owned(),
    }
}

/// `<` / `>` collapse button at a column edge (TRKR `panel_toggle_button`).
pub fn panel_toggle(
    ui: &mut egui::Ui,
    r: Rect,
    pointing_right: bool,
    enabled: bool,
    hover: &str,
) -> bool {
    ui.add_enabled_ui(enabled, |ui| {
        ui.put(
            r,
            egui::Button::new(if pointing_right { ">" } else { "<" })
                .min_size(r.size())
                .corner_radius(4),
        )
    })
    .inner
    .on_hover_text(hover)
    .on_disabled_hover_text(hover)
    .clicked()
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    #[test]
    fn startup_fit_for_common_screens() {
        let request = Vec2::new(1566., 900.);
        // 1920×1080 at 100 %: fits, full layout.
        let f = fit_window(Vec2::new(1920., 1080.), request);
        assert!(!f.maximize && !f.collapse_right && !f.collapse_left);
        assert_eq!(f.min_inner, Vec2::new(1566., 700.));
        // 1920×1080 at 125 % (1536×864 points): maximize, log column folded.
        let f = fit_window(Vec2::new(1536., 864.), request);
        assert!(f.maximize && f.collapse_right && !f.collapse_left);
        assert_eq!(f.min_inner, Vec2::new(1293., 700.));
        // RDP 1600×900: width fits, 900 does not → maximize, full layout.
        let f = fit_window(Vec2::new(1600., 900.), request);
        assert!(f.maximize && !f.collapse_right);
        assert_eq!(f.min_inner, Vec2::new(1566., 700.));
        // 1366×768: maximize, log folded, min height clamped to the screen.
        let f = fit_window(Vec2::new(1366., 768.), request);
        assert!(f.maximize && f.collapse_right && !f.collapse_left);
        assert_eq!(f.min_inner, Vec2::new(1293., 672.));
        // 1280×720 at 150 % (853×480): both side columns folded; the
        // minimum never exceeds the usable area.
        let f = fit_window(Vec2::new(853., 480.), request);
        assert!(f.maximize && f.collapse_right && f.collapse_left);
        assert!(f.min_inner.x <= f.usable.x && f.min_inner.y <= f.usable.y);
    }
}
