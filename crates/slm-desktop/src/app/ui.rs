//! Window layout following KohdaLab TRKR: left Config + SLM device cards,
//! center tabs with one operation card (form | Output + Run) above the shared
//! experiment list, right Log above a Field/Value table of the applied
//! condition, `<`/`>` collapse buttons at the column edges.
//!
//! Only presentation lives here; actions and worker handling are in `app.rs`
//! and its `device`, `experiment` and `preview_window` modules. This file
//! keeps the top-level layout; each area is drawn by a submodule.

use super::*;
use crate::layout::{Columns, panel_toggle};
use crate::theme::BORDER;
use eframe::egui::{Color32, Pos2, Rect, Vec2};

mod device_panel;
mod experiment_list;
mod forms;
mod input_preview;
mod log_applied;
mod operation;

const GREEN: Color32 = Color32::from_rgb(100, 196, 146);
const HEADER_FILL: Color32 = Color32::from_rgb(45, 46, 52);
const ROW: f32 = 32.;
const H: f32 = 26.;
/// Prefix of every External control state label in the tab row.
#[cfg(test)]
pub(crate) const EXTERNAL_CONTROL: &str = "External control";
const EXTERNAL_AVAILABLE: &str = "External control · Available";
pub(crate) const EXTERNAL_LEASED: &str = "External control · Python";
const EXTERNAL_WAITING: &str = "External control · Waiting for device";
const EXTERNAL_UNVERIFIED: &str = "External control · Output unverified";
const EXTERNAL_AVAILABLE_TIP: &str = "Started with --gui-api. A local client can acquire a lease to connect to the selected display, apply patterns, reset and stop. While it holds the lease, manual output controls here are locked; Stop and Take control stay available.";
const EXTERNAL_LEASED_TIP: &str = "A local client holds the SLM lease. Stop: end the lease, send the reset pattern, then close the output. Take control: end the lease and leave the output as it is.";
const EXTERNAL_WAITING_TIP: &str = "Waiting for the device to acknowledge the last external operation or its stop. A new lease is refused until then.";
const EXTERNAL_UNVERIFIED_TIP: &str = "The SLM output is unverified. A new lease is refused until a backend-confirmed Connect in this window.";

impl App {
    pub(super) fn draw(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND))
            .show_inside(root, |ui| {
                let viewport = ui.max_rect();
                let minimum = Columns::minimum_width(self.right_collapsed, self.left_collapsed);
                // Layout minimum, capped by the usable screen once known.
                let mut min = Vec2::new(minimum, crate::layout::MIN_HEIGHT);
                if let Some(limit) = self.window_fit.limit {
                    min = min.min(limit);
                }
                // Wait for the startup fit, so a small screen never sees the
                // full-layout minimum before the window is fitted.
                if self.window_fit.done || self.capture.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
                }
                let left_collapsed = self.left_collapsed;
                let right_collapsed = self.right_collapsed;
                let columns = Columns::new(viewport, right_collapsed, left_collapsed);
                let available_width = self.window_fit.limit.map_or(f32::INFINITY, |size| size.x);
                let both_fit = Columns::minimum_width(false, false) <= available_width;
                let left_enabled =
                    !left_collapsed || Columns::minimum_width(true, false) <= available_width;
                let right_enabled =
                    !right_collapsed || Columns::minimum_width(false, true) <= available_width;
                if !left_collapsed {
                    self.left_rail(ui, &columns);
                }
                ui.painter().line_segment(
                    [
                        columns.left.right_top() + Vec2::new(4., 0.),
                        columns.left.right_bottom() + Vec2::new(4., 0.),
                    ],
                    egui::Stroke::new(1_f32, BORDER),
                );
                let left_tip = if !left_collapsed {
                    "Hide device panel"
                } else if left_enabled {
                    "Show device panel"
                } else {
                    "The device panel needs a wider screen"
                };
                if panel_toggle(
                    ui,
                    columns.left_toggle,
                    left_collapsed,
                    left_enabled,
                    left_tip,
                ) {
                    self.left_collapsed = !left_collapsed;
                    self.right_collapsed |= left_collapsed && !both_fit;
                }
                let right_tip = if !right_collapsed {
                    "Hide log"
                } else if right_enabled {
                    "Show log"
                } else {
                    "The log needs a wider screen"
                };
                if panel_toggle(
                    ui,
                    columns.toggle,
                    !right_collapsed,
                    right_enabled,
                    right_tip,
                ) {
                    self.right_collapsed = !right_collapsed;
                    self.left_collapsed |= right_collapsed && !both_fit;
                }
                if !right_collapsed {
                    self.log_view(ui, columns.log);
                    self.applied_table(ui, columns.table);
                }
                self.operation(ui, columns.measurement);
                let bottom = Rect::from_min_max(columns.toolbar.min, columns.list.max);
                self.input_pattern(ui, bottom);
                if self.settings_open {
                    self.device_settings_window(
                        ui.ctx(),
                        Pos2::new(columns.measurement.right() - 378., bottom.top() + 62.),
                    );
                }
            });
    }

    // ----- left rail -----------------------------------------------------

    fn left_rail(&mut self, ui: &mut egui::Ui, columns: &Columns) {
        let content = Rect::from_min_max(
            columns.left.min + Vec2::new(2., 2.),
            Pos2::new(columns.left_toggle.min.x - 4., columns.left.max.y - 2.),
        );
        let origin = content.min + Vec2::new(9., 9.);
        let width = content.width() - 18.;
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(content));
            let config_h = self.config_card(ui, origin, width);
            let area = Rect::from_min_max(
                Pos2::new(content.min.x, origin.y + config_h + 8.),
                content.max,
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(area));
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();
                    egui::ScrollArea::vertical()
                        .id_salt("device-scroll")
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                        )
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let top = ui.max_rect().min + Vec2::new(9., 0.);
                            let slm = self.slm_card(ui, top, width);
                            let height = (area.height() - slm - 18.).floor().max(196.);
                            self.list_card(ui, top + Vec2::new(0., slm + 8.), width, height);
                            ui.set_min_size(Vec2::new(ui.available_width(), slm + height + 16.));
                        });
                },
            );
        });
    }
}

/// `~/…` display form of a path, as TRKR shows its config path.
pub(super) fn tilde(path: &std::path::Path) -> String {
    match slm_device::config::home_dir()
        .and_then(|home| path.strip_prefix(&home).ok().map(PathBuf::from))
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn expand_home(text: &str) -> PathBuf {
    match (text.strip_prefix("~/"), slm_device::config::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(text),
    }
}
