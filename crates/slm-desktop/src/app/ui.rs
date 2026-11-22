//! Window layout following KohdaLab TRKR: left Config + SLM device cards,
//! center tabs with one operation card (form | Output + Run) above the shared
//! experiment list, right Log above a Field/Value table of the applied
//! condition, `<`/`>` collapse buttons at the column edges.
//!
//! Only presentation lives here; actions and worker handling are in `app.rs`.

use super::*;
use crate::layout::{Columns, Form, Tone, panel_toggle, rect};
use crate::theme::{BORDER, TEXT};
use eframe::egui::{Color32, Pos2, Rect, Vec2};
use slm_core::heatmap::Colormap;

const GREEN: Color32 = Color32::from_rgb(100, 196, 146);
const HEADER_FILL: Color32 = Color32::from_rgb(45, 46, 52);
const ROW: f32 = 32.;
const H: f32 = 26.;

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
        if let Some((at, level, text)) = &self.notice
            && at.elapsed() < std::time::Duration::from_secs(7)
        {
            egui::Area::new(egui::Id::new("operation-notice"))
                .anchor(egui::Align2::RIGHT_BOTTOM, [-16., -16.])
                .interactable(false)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(420.);
                        ui.colored_label(
                            match level {
                                Level::Error => ERROR,
                                Level::Warn => AMBER,
                                Level::Info => TEAL,
                            },
                            text,
                        );
                    });
                });
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        if !ctx
            .memory(|m| m.focused())
            .is_some_and(|id| egui::TextEdit::load_state(&ctx, id).is_some())
            && !self.settings_open
            && !self.confirm_close
            && !self.shut_down
            && self.pending_config_load.is_none()
        {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
                && let Some(path) = self
                    .experiment
                    .path
                    .clone()
                    .or_else(|| self.pick_save_path())
            {
                self.save_experiment(&path);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
                && self.apply_block().is_none()
                && let Some(index) = self.selected
            {
                self.apply_experiment_item(index);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.undo_remove();
            }
        }
        self.unsaved_dialogs(&ctx);
    }

    fn unsaved_dialogs(&mut self, ctx: &egui::Context) {
        if self.shut_down && !self.shutdown_done {
            let mut force = false;
            egui::Modal::new(egui::Id::new("shutdown-progress")).show(ctx, |ui| {
                ui.heading("Closing output session");
                if let Some(error) = &self.shutdown_error {
                    ui.colored_label(ERROR, error);
                    ui.label("Check the SLM before continuing an experiment. Closing this app does not prove the optical state.");
                    force = ui.button("Close Anyway").clicked();
                } else { ui.label("Resetting output before close…"); ui.spinner(); }
            });
            if force {
                self.journal_event(
                    "shutdown_forced",
                    serde_json::json!({"error":self.shutdown_error}),
                );
                self.shutdown_done = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if self.confirm_close {
            let response = egui::Modal::new(egui::Id::new("unsaved-close")).show(ctx, |ui| {
                ui.heading("Save changes before closing?");
                if self.experiment.dirty {
                    ui.label("The pattern list has unsaved changes.");
                }
                if !self.demo && self.config_dirty {
                    ui.label("The configuration has unsaved changes.");
                }
                if let Some(last) = self
                    .log
                    .back()
                    .filter(|line| matches!(line.level, Level::Error))
                {
                    ui.colored_label(ERROR, &last.text);
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        return 1;
                    }
                    if ui.button("Discard and Close").clicked() {
                        return 2;
                    }
                    if ui.button("Save and Close").clicked() {
                        return 3;
                    }
                    0
                })
                .inner
            });
            let action = response.inner;
            if action == 1 || response.should_close() {
                self.confirm_close = false;
            }
            if action == 2 || (action == 3 && self.save_changes_before_close()) {
                self.confirm_close = false;
                self.close_allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if let Some(path) = self.pending_config_load.clone() {
            let response = egui::Modal::new(egui::Id::new("unsaved-config-load")).show(ctx, |ui| {
                ui.heading("Replace unsaved configuration?");
                ui.label("Load will discard the current configuration changes.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        return 1;
                    }
                    if ui.button("Discard and Load").clicked() {
                        return 2;
                    }
                    0
                })
                .inner
            });
            if response.inner == 2 {
                self.load_config(&path);
            }
            if response.inner != 0 || response.should_close() {
                self.pending_config_load = None;
            }
        }
    }

    fn save_changes_before_close(&mut self) -> bool {
        if self.experiment.dirty {
            let Some(path) = self
                .experiment
                .path
                .clone()
                .or_else(|| self.pick_save_path())
            else {
                return false;
            };
            if !self.save_experiment(&path) {
                return false;
            }
        }
        if !self.demo && self.config_dirty {
            let path = expand_home(self.config_path_text.trim());
            if !self.save_config_to(&path) {
                return false;
            }
        }
        true
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

    fn config_card(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) -> f32 {
        let inner = w - 22.;
        let half = ((inner - 8.) / 2.).round();
        let busy = self.device_busy();
        let (browse, load, save);
        {
            let mut f = Form::new(ui, origin);
            f.group(
                rect(0., 0., w, 96.),
                if self.config_dirty {
                    "Config · unsaved"
                } else {
                    "Config"
                },
            );
            f.path(
                "config-path",
                rect(11., 27., inner - 78., H),
                &mut self.config_path_text,
            );
            browse = f.button(rect(w - 81., 27., 70., H), "Browse", !self.demo);
            load = f.button_with(
                rect(11., 59., half, H),
                "Load",
                !busy && !self.demo,
                Tone::Neutral,
                Some(if self.demo {
                    "Demo uses a temporary config"
                } else {
                    "Disconnect before loading another config"
                }),
            );
            save = f.button_with(
                rect(19. + half, 59., inner - 8. - half, H),
                "Save",
                !self.demo && !self.wavelength_busy,
                Tone::Neutral,
                Some(if self.demo {
                    "Demo config is temporary"
                } else if self.wavelength_busy {
                    "Wait for the wavelength operation to finish"
                } else {
                    "Save profile and output settings"
                }),
            );
        }
        if browse
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("KohdaLab SLM config", &["json"])
                .pick_file()
        {
            self.config_path_text = p.display().to_string();
        }
        if load {
            let path = expand_home(self.config_path_text.trim());
            if self.config_dirty {
                self.pending_config_load = Some(path);
            } else {
                self.load_config(&path);
            }
        }
        if save {
            let path = expand_home(self.config_path_text.trim());
            self.save_config_to(&path);
        }
        96.
    }

    fn state_caption(&self) -> (String, Color32) {
        let simulated = self.config.output.backend == BackendKind::Demo;
        match &self.device_state {
            DeviceState::Disconnected => ("----".into(), MUTED),
            DeviceState::Connecting => ("Connecting…".into(), AMBER),
            DeviceState::Connected { display, mode, .. } => (
                format!(
                    "{} · {}{}",
                    display.gdi_name.trim_start_matches(r"\\.\"),
                    mode.short(),
                    if simulated { " · SIM" } else { "" }
                ),
                TEAL,
            ),
            DeviceState::Unknown { .. } => ("Output unknown".into(), ERROR),
        }
    }

    /// SLM device card on TRKR's device-card grid: Device disclosure with
    /// status, a three-column grid, and a separate settings window.
    fn slm_card(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) -> f32 {
        let busy = self.device_busy();
        let connected = self.connected();
        let (caption, caption_color) = self.state_caption();
        let block = self.connect_block();
        // Height is known only after laying out; paint the card first with the
        // previous frame's height so content draws on top of it.
        let id = egui::Id::new("slm-card-height");
        let height = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(160.);
        let lx = 11.;
        let col_width = (w - 34.) / 3.;
        let fx = lx + col_width + 6.;
        let action_x = fx + col_width + 6.;
        let field_w = w - lx - fx;
        let mut f = Form::new(ui, origin);
        f.group(
            rect(0., 0., w, height),
            &format!("SLM · {}", self.config.profile.model.label()),
        );
        f.toggle(
            rect(11., 27., 71., 24.),
            &mut self.device_expanded,
            "Device",
        );
        f.dot(
            Pos2::new(93., 39.),
            if connected {
                GREEN
            } else if matches!(self.device_state, DeviceState::Unknown { .. }) {
                ERROR
            } else {
                Color32::GRAY
            },
        );
        let caption = if matches!(self.device_state, DeviceState::Disconnected) {
            "Disconnected".into()
        } else {
            caption
        };
        f.value(rect(102., 27., w - 113., 24.), caption, caption_color);
        let mut y = 59.;

        if self.device_expanded {
            y = 66.;
            f.enabled = !busy;
            f.label(rect(lx, y, col_width, H), "Backend");
            let mut backend = self.config.output.backend;
            let options: Vec<(BackendKind, String, bool)> = BackendKind::available()
                .iter()
                .filter(|k| !self.demo || **k == BackendKind::Demo)
                .map(|k| (*k, k.label().to_string(), true))
                .collect();
            if f.combo("backend", rect(fx, y, field_w, H), &mut backend, &options) {
                self.config.output.backend = backend;
                self.wavelength_nm =
                    Self::initial_wavelength_nm(self.config.profile.device_wavelength_nm);
                self.wavelength_status = None;
                self.config.output.display = None;
                self.config.output.confirmed_mode = None;
                self.mode_ack = false;
                self.displays.clear();
                self.refresh_displays();
                self.save_config();
            }
            y += ROW;
            f.label(rect(lx, y, col_width, H), "Mode");
            match self.config.input_mode_confirmation().cloned() {
                Some(c) => {
                    f.value(
                        rect(fx, y, col_width, H),
                        format!("{} · confirmed", c.mode.short()),
                        TEAL,
                    );
                    if f.button_with(
                        rect(action_x, y, col_width, H),
                        "Clear",
                        true,
                        Tone::Neutral,
                        Some(&c.note),
                    ) {
                        self.config.output.confirmed_mode = None;
                        self.push_log(
                            Level::Info,
                            "Input-mode confirmation cleared; output is blocked.",
                        );
                        self.save_config();
                    }
                    y += ROW;
                }
                None => {
                    let options: Vec<(InputMode, String, bool)> = InputMode::ALL
                        .iter()
                        .map(|m| (*m, m.label().to_string(), m.supported()))
                        .collect();
                    f.combo(
                        "mode-choice",
                        rect(fx, y, field_w, H),
                        &mut self.mode_choice,
                        &options,
                    );
                    y += ROW;
                    f.label(rect(lx, y, col_width, H), "Note");
                    f.text("mode-note", rect(fx, y, field_w, H), &mut self.mode_note)
                        .on_hover_text("What was checked on the SLM (menu, test codes)");
                    y += ROW;
                    f.check(
                        rect(lx, y, action_x - lx - 6., H),
                        &mut self.mode_ack,
                        "Checked on SLM",
                    );
                    let target = self
                        .config
                        .output
                        .display
                        .as_ref()
                        .filter(|d| !d.monitor_device_path.is_empty());
                    let ok = self.mode_ack
                        && self.mode_choice.supported()
                        && !self.mode_note.trim().is_empty()
                        && target.is_some();
                    if f.button_with(
                        rect(action_x, y, col_width, H),
                        "Confirm",
                        ok,
                        Tone::Neutral,
                        Some(if target.is_some() {
                            "Record the verified input mode (60 Hz, Table 6)"
                        } else {
                            "Select the SLM display first"
                        }),
                    ) && let Some(target) = target
                    {
                        self.config.output.confirmed_mode = Some(ModeConfirmation {
                            monitor_device_path: target.monitor_device_path.clone(),
                            mode: self.mode_choice,
                            confirmed_unix_s: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_secs()),
                            note: self.mode_note.trim().to_string(),
                        });
                        self.mode_ack = false;
                        self.push_log(
                            Level::Info,
                            format!("Input mode confirmed: {}", self.mode_choice.short()),
                        );
                        self.save_config();
                    }
                    y += ROW;
                }
            }
            if self.config.output.backend == BackendKind::Demo {
                // Demo-only simulation of Windows display events, on the
                // card's full-width three-button grid.
                f.enabled = true;
                f.muted(rect(lx, y, w - 31., 18.), "Simulate (demo only)");
                y += 22.;
                let bw = ((w - 34.) / 3.).floor();
                let mut action = None;
                for (i, name) in ["Renumber", "Unplug", "Duplicate", "Fail Draw", "Restore"]
                    .iter()
                    .enumerate()
                {
                    let (col, row) = (i % 3, i / 3);
                    let r = rect(11. + col as f32 * (bw + 6.), y + row as f32 * ROW, bw, H);
                    if f.button(r, name, true) {
                        action = Some(*name);
                    }
                }
                y += 2. * ROW;
                if let Some(name) = action {
                    self.demo_action(name);
                }
            }
        }

        f.enabled = !busy;
        f.label(rect(lx, y, col_width, H), "Display");
        let current = self.config.output.display.clone();
        let index = current.as_ref().and_then(|s| {
            self.displays.iter().position(|d| {
                d.monitor_device_path == s.monitor_device_path && d.gdi_name == s.gdi_name
            })
        });
        let mut choice = index.unwrap_or(usize::MAX);
        // Short closed text (number · name); full details in the list / hover.
        let short = |gdi: &str, name: &str| {
            let display = gdi.trim_start_matches(r"\\.\");
            let number = display.strip_prefix("DISPLAY").unwrap_or(display);
            format!("{number} · {}", name.trim_end_matches(" (demo)"))
        };
        let closed = match (&current, index) {
            (None, _) => "Select display…".to_string(),
            (Some(s), Some(_)) => short(&s.gdi_name, &s.friendly_name),
            (Some(s), None) => format!("{} (not found)", short(&s.gdi_name, &s.friendly_name)),
        };
        let mut options: Vec<(usize, String, bool)> =
            vec![(usize::MAX, "Select display…".into(), false)];
        options.extend(
            self.displays
                .iter()
                .enumerate()
                .map(|(i, d)| (i, d.label(), true)),
        );
        let full = self
            .displays
            .get(choice)
            .map_or_else(|| "No display selected".into(), |d| d.label());
        let display_rect = rect(fx, y, col_width, H);
        f.ui.interact(
            f.at(display_rect),
            egui::Id::new("output-display-hover"),
            egui::Sense::hover(),
        )
        .on_hover_text(full);
        if f.combo_closed(
            "output-display",
            display_rect,
            &mut choice,
            &options,
            Some(closed),
        ) && let Some(d) = self.displays.get(choice)
        {
            let selection = d.selection();
            self.push_log(
                Level::Info,
                format!("Output display: {} (nothing sent).", selection.label()),
            );
            self.config.output.display = Some(selection);
            if self.config.input_mode_confirmation().is_none() {
                self.config.output.confirmed_mode = None;
                self.mode_ack = false;
            }
            self.save_config();
        }
        if f.button_with(
            rect(action_x, y, col_width, H),
            "Refresh",
            !busy,
            Tone::Neutral,
            Some("Re-list Windows displays"),
        ) {
            self.refresh_displays();
        }
        y += ROW;

        f.enabled = true;
        let connect_reason = block
            .clone()
            .unwrap_or_else(|| "Show the reset pattern on the selected display".into());
        let connect = f.button_with(
            rect(lx, y, col_width, H),
            "Connect",
            matches!(self.device_state, DeviceState::Disconnected) && block.is_none(),
            Tone::Neutral,
            Some(&connect_reason),
        );
        let disconnect = f.button_with(
            rect(fx, y, col_width, H),
            "Disconnect",
            matches!(
                self.device_state,
                DeviceState::Connected { .. } | DeviceState::Unknown { .. }
            ) && self.pending.is_none(),
            Tone::Neutral,
            Some("Send the reset pattern, then close the output"),
        );
        let settings = f.button_with(
            rect(action_x, y, col_width, H),
            "Settings",
            true,
            Tone::Neutral,
            Some("Show or hide the device settings window"),
        );
        y += ROW;

        f.label(rect(lx, y, col_width, H), "Reset");
        f.value(
            rect(fx, y, col_width, H),
            format!("code {}", self.config.profile.reset_code),
            TEXT,
        );
        let reset = f.button_with(
            rect(action_x, y, col_width, H),
            "Reset Pattern",
            connected && self.pending.is_none(),
            Tone::Neutral,
            Some(&format!(
                "Show uniform code {} now",
                self.config.profile.reset_code
            )),
        );
        y += ROW;

        let reason = match &self.device_state {
            DeviceState::Disconnected => block.map(|b| (b, AMBER)),
            DeviceState::Unknown { reason } => Some((reason.clone(), ERROR)),
            _ => None,
        };
        if let Some((text, color)) = reason {
            f.note(rect(11., y - 4., w - 22., 18.), &text, color);
            y += 16.;
        }
        if let Some(e) = &self.displays_error {
            f.note(rect(11., y - 4., w - 22., 18.), e, ERROR);
            y += 16.;
        }
        let height = y + 4.;
        ui.data_mut(|d| d.insert_temp(id, height));
        if connect {
            self.connect();
        }
        if disconnect {
            self.disconnect();
        }
        if settings {
            self.settings_open = !self.settings_open;
            if !self.settings_open {
                self.settings_draft = None;
            }
        }
        if reset {
            self.reset_pattern();
        }
        height
    }

    fn demo_action(&mut self, name: &str) {
        let mut displays = demo_displays();
        match name {
            "Renumber" => {
                displays[1].gdi_name = r"\\.\DISPLAY3".into();
                self.demo_control
                    .simulate_display_change(displays, "Windows renumbered the displays.");
            }
            "Unplug" => {
                self.demo_control.simulate_display_change(
                    vec![displays[0].clone()],
                    "The SLM display disappeared.",
                );
            }
            "Duplicate" => {
                displays[1].clone_count = 2;
                self.demo_control
                    .simulate_display_change(displays, "Windows switched to duplicate mode.");
            }
            "Fail Draw" => {
                self.demo_control.lock().fail_next_show = Some("Simulated draw failure".into())
            }
            _ => self.demo_control.lock().displays = displays,
        }
        self.refresh_displays();
    }

    fn device_settings_window(&mut self, ctx: &egui::Context, origin: Pos2) {
        let mut open = true;
        let mut accept = false;
        let mut cancel = false;
        let width = 380.;
        let actual = self.config.profile.clone();
        let dirty = self.config_dirty;
        let mut draft = self.settings_draft.take().unwrap_or_else(|| actual.clone());
        draft.device_wavelength_nm = actual.device_wavelength_nm;
        draft.device_phase_raw = actual.device_phase_raw;
        self.config.profile = draft;
        egui::Window::new("Device Settings")
            .id(egui::Id::new("device-settings-window"))
            .default_pos(Pos2::new(origin.x, 60.))
            .default_width(width)
            .resizable(false)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 220.).max(150.))
                    .show(ui, |ui| {
                        let start = ui.available_rect_before_wrap().min;
                        let mut form = Form::new(ui, start);
                        let used = self.settings_rows(&mut form, width, 4.);
                        ui.set_min_size(Vec2::new(width, used));
                    });
                ui.label("USB Read/Write and Copy center act immediately.");
                ui.horizontal(|ui| {
                    accept = ui
                        .add_enabled(
                            !self.device_busy() && self.config.profile.validate().is_ok(),
                            egui::Button::new("Use Settings"),
                        )
                        .clicked();
                    cancel = ui.button("Cancel Settings").clicked();
                });
            });
        let draft = self.config.profile.clone();
        let hardware_changed = actual.device_wavelength_nm != draft.device_wavelength_nm
            || actual.device_phase_raw != draft.device_phase_raw;
        self.config.profile = actual;
        self.config.profile.device_wavelength_nm = draft.device_wavelength_nm;
        self.config.profile.device_phase_raw = draft.device_phase_raw;
        self.config_dirty = dirty || hardware_changed;
        if accept {
            self.config_dirty |= self.config.profile != draft;
            self.config.profile = draft;
            self.settings_open = false;
        } else if !open || cancel {
            self.settings_open = false;
        } else {
            self.settings_draft = Some(draft);
        }
    }

    fn settings_rows(&mut self, f: &mut Form, w: f32, mut y: f32) -> f32 {
        let locked = self.device_busy();
        let before = self.config.clone();
        f.note(
            rect(11., y, w - 22., H),
            if locked {
                "Disconnect to edit · saved by Config Save"
            } else {
                "Use Settings, then Config Save to persist"
            },
            MUTED,
        );
        y += ROW;
        y = self.wavelength_rows(f, w, y, locked);
        f.enabled = !locked;
        let p = &mut self.config.profile;
        let (lx, fx, fw, ux) = (20., 114., 96., 216.);
        let row = |f: &mut Form,
                   label: &str,
                   unit: &str,
                   y: &mut f32,
                   edit: &mut dyn FnMut(&mut Form, Rect)| {
            f.label(rect(lx, *y, fx - lx - 6., H), label);
            edit(f, rect(fx, *y, fw, H));
            f.muted(rect(ux, *y, w - ux - 11., H), unit);
            *y += ROW;
        };
        row(f, "Pixel pitch", "µm/px", &mut y, &mut |f, r| {
            f.number("pitch", r, &mut p.generation.optics.pixel_pitch_um, true);
        });
        row(f, "Demag. 1/M", "×", &mut y, &mut |f, r| {
            let mut demag = 1. / p.generation.optics.magnification;
            if f.number("demag", r, &mut demag, true) && demag > 0. {
                p.generation.optics.magnification = 1. / demag;
            }
        });
        use slm_core::PhaseMapping;
        let calibration = &mut p.generation.calibration;
        f.label(rect(lx, y, fx - lx - 6., H), "Phase mapping");
        f.combo(
            "phase-mapping",
            rect(fx, y, w - fx - 11., H),
            &mut calibration.mapping,
            &[
                (
                    PhaseMapping::SantecFullScale,
                    "Santec full scale (2π)".into(),
                    true,
                ),
                (PhaseMapping::Notebook, "Notebook calibration".into(), true),
            ],
        );
        y += ROW;
        match calibration.mapping {
            PhaseMapping::Notebook => {
                row(f, "phase0", "code", &mut y, &mut |f, r| {
                    f.number("phase0", r, &mut calibration.phase0, true);
                });
                row(f, "Code period", "codes / 2π", &mut y, &mut |f, r| {
                    f.integer("code-period", r, &mut calibration.code_period, 1., 1024.);
                });
            }
            PhaseMapping::SantecFullScale => {
                f.note(
                    rect(lx, y, w - lx - 11., H),
                    "Set a 2π phase range in the Santec software.",
                    MUTED,
                );
                y += ROW;
            }
        }
        row(f, "Mount angle", "°", &mut y, &mut |f, r| {
            f.number("mount", r, &mut p.generation.mount_angle_deg, true);
        });
        let max = f64::from(p.generation.max_code);
        row(f, "Reset code", "code", &mut y, &mut |f, r| {
            f.integer("reset-code", r, &mut p.reset_code, 0., max);
        });
        row(f, "Band width", "px default", &mut y, &mut |f, r| {
            f.integer(
                "band-default",
                r,
                &mut p.generation.default_stripe_width_px,
                1.,
                10_000.,
            );
        });
        row(f, "Default center", "px", &mut y, &mut |f, r| {
            let half = (r.width() - 4.) / 2.;
            let mut cx = p.default_center_offset_px[0];
            let mut cy = p.default_center_offset_px[1];
            f.integer(
                "center-x",
                rect(r.min.x, r.min.y, half, H),
                &mut cx,
                -100_000.,
                100_000.,
            );
            f.integer(
                "center-y",
                rect(r.min.x + half + 4., r.min.y, half, H),
                &mut cy,
                -100_000.,
                100_000.,
            );
            p.default_center_offset_px = [cx, cy];
        });
        if f.button(
            rect(lx, y, w - lx - 11., H),
            "Copy default center to forms",
            !locked,
        ) {
            self.single.geometry.center_offset_px = p.default_center_offset_px;
            self.multi.geometry.center_offset_px = p.default_center_offset_px;
        }
        y += ROW;
        f.label(rect(lx, y, fx - lx - 6., H), "Note");
        f.text(
            "calibration-note",
            rect(fx, y, w - fx - 11., H),
            &mut p.calibration_note,
        )
        .on_hover_text(p.calibration_note.clone());
        y += ROW;
        f.enabled = true;
        if let Err(e) = self.config.profile.validate() {
            f.note(rect(11., y - 4., w - 22., 18.), &e, ERROR);
            y += 16.;
        }
        if self.config != before {
            self.config_dirty = true;
        }
        y
    }

    fn wavelength_rows(&mut self, f: &mut Form, w: f32, mut y: f32, locked: bool) -> f32 {
        let simulated = self.config.output.backend == BackendKind::Demo;
        f.enabled = !locked;
        f.label(rect(20., y, 89., H), "Wavelength");
        if f.text("wavelength", rect(114., y, 86., H), &mut self.wavelength_nm)
            .changed()
        {
            self.wavelength_status = None;
        }
        f.muted(rect(206., y, 22., H), "nm");
        if !simulated {
            f.label(rect(232., y, 46., H), "USB #");
            if f.text(
                "wavelength-device",
                rect(279., y, w - 290., H),
                &mut self.wavelength_usb_device,
            )
            .changed()
            {
                self.wavelength_status = None;
            }
        }
        y += ROW;
        let half = (w - 28.) / 2.;
        let device_ok = self.wavelength_device_number().is_some();
        let valid = self.wavelength_target().is_some();
        let read = f.button(rect(11., y, half, H), "Read Wavelength", device_ok);
        let write = f.button_with(
            rect(17. + half, y, half, H),
            "Set Wavelength",
            device_ok && valid,
            Tone::Primary,
            Some(if valid {
                "Write wavelength, preserve the current phase range, and verify by readback"
            } else {
                "Enter an integer from 450 to 1600 nm"
            }),
        );
        f.enabled = true;
        if read || write {
            self.request_wavelength(write);
        }
        y += ROW;
        let (status, color) = match &self.wavelength_status {
            Some((status, color)) => (status.as_str(), *color),
            None if simulated => ("Simulated USB calibration", MUTED),
            None => (
                "Santec USB calibration · disconnect video output first",
                MUTED,
            ),
        };
        f.note(rect(20., y, w - 31., 22.), status, color);
        y + 28.
    }

    // ----- center --------------------------------------------------------

    fn operation(&mut self, ui: &mut egui::Ui, area: Rect) {
        let widths = [74., 92., 86., 62., 88.];
        let mut x = area.min.x;
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            let r = rect(x, area.min.y, widths[i], 24.);
            if ui
                .put(
                    r,
                    egui::Button::new(tab.label())
                        .selected(self.tab == tab)
                        .min_size(r.size())
                        .corner_radius(6),
                )
                .clicked()
                && self.tab != tab
            {
                self.tab = tab;
                self.unit_error = None;
            }
            x += widths[i];
        }
        let page = rect(
            area.min.x + 2.,
            area.min.y + 26.,
            area.width() - 6.,
            area.height() - 30.,
        );
        ui.painter().rect(
            page,
            10.,
            theme::PANEL,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let side = 250_f32.min((page.width() - 8. - 216.).max(180.));
        let settings = page.width() - 24. - side;
        let side_x = settings + 12.;
        match self.tab {
            Tab::Uniform => self.uniform_form(ui, page.min, settings),
            Tab::Single => self.single_form(ui, page.min, settings),
            Tab::Multi => self.multi_form(ui, page.min, settings),
            Tab::Sweep => self.sweep_form(ui, page.min, settings),
            Tab::Experiment => self.experiment_form(ui, page.min, settings),
        }
        self.output_run(ui, page.min, side_x, side);
    }

    fn form_spec(&self) -> Option<(String, PatternSpec)> {
        match self.tab {
            Tab::Uniform => Some((
                format!("Uniform code {}", self.uniform_code),
                self.uniform_spec(),
            )),
            Tab::Single => Some(("Single Wave".into(), self.single_spec())),
            Tab::Multi => Some(("Multi-Wave".into(), self.multi_spec())),
            Tab::Sweep | Tab::Experiment => None,
        }
    }

    fn output_run(&mut self, ui: &mut egui::Ui, origin: Pos2, side_x: f32, side: f32) {
        let profile = self.config.profile.generation;
        let form = self.form_spec();
        let valid = form.as_ref().map(|(_, s)| s.validate(&profile));
        let block = self.apply_block();
        let display = self.config.output.display.as_ref().map_or_else(
            || "Not selected".into(),
            |s| {
                format!(
                    "{} · {}",
                    s.gdi_name.trim_start_matches(r"\\.\"),
                    s.friendly_name
                )
            },
        );
        let (state, color) = match &self.device_state {
            DeviceState::Disconnected => ("Disconnected".to_string(), MUTED),
            DeviceState::Connecting => ("Connecting…".into(), AMBER),
            DeviceState::Connected { mode, .. } => (
                format!(
                    "Connected · {}{}",
                    mode.short(),
                    if self.config.output.backend == BackendKind::Demo {
                        " · simulated"
                    } else {
                        ""
                    }
                ),
                TEAL,
            ),
            DeviceState::Unknown { .. } => ("Output unknown".into(), ERROR),
        };
        let sweep = (self.tab == Tab::Sweep).then(|| self.sweep_validation());
        let mut f = Form::new(ui, origin);
        f.group(rect(side_x, 12., side, 82.), "Output");
        f.label(rect(side_x + 11., 34., 44., H), "Display");
        f.note(rect(side_x + 61., 34., side - 72., H), &display, TEXT);
        f.label(rect(side_x + 11., 61., 44., H), "State");
        f.note(rect(side_x + 61., 61., side - 72., H), &state, color);

        let x = side_x + 11.;
        let width = side - 22.;
        let half = ((width - 6.) / 2.).round();
        let preview = self.current_preview();
        let can_save = preview
            .as_ref()
            .is_some_and(|(key, _, _)| self.preview.shown.as_deref() == Some(key.as_str()));
        let target = match preview.as_ref() {
            Some((_, crate::preview::PreviewSource::Item { item, .. }, _)) => self
                .selected
                .map_or_else(|| item.label.clone(), |i| self.row_caption(i)),
            Some((_, _, caption)) if self.tab == Tab::Sweep => {
                if caption.starts_with("Sweep first value") {
                    "Sweep first value".into()
                } else {
                    "Sweep base".into()
                }
            }
            Some((_, _, caption)) => caption.clone(),
            None => "Select a pattern".into(),
        };
        f.group(rect(side_x, 102., side, 102.), "Run");
        if let Some(sweep) = sweep {
            if f.button_with(
                rect(x, 125., width, H),
                "Add Sweep",
                sweep.is_ok(),
                Tone::Primary,
                Some("Append all sweep values to Patterns"),
            ) {
                self.add_sweep();
            }
        } else if let Some((label, spec)) = form {
            let valid = matches!(valid, Some(Ok(())));
            if f.button_with(
                rect(x, 125., half, H),
                "Apply",
                block.is_none() && valid,
                Tone::Primary,
                Some(block.unwrap_or("Apply the current form")),
            ) {
                self.apply_spec(label.clone(), spec.clone());
            }
            if f.button_with(
                rect(x + half + 6., 125., width - half - 6., H),
                "Add to List",
                valid,
                Tone::Neutral,
                Some("Append this form to Patterns"),
            ) {
                self.add_to_experiment(label, spec);
            }
        }
        let save_y = if self.tab == Tab::Experiment {
            146.
        } else {
            172.
        };
        f.note(
            rect(x, save_y - 21., width, 19.),
            &format!("Save target · {target}"),
            MUTED,
        );
        for (png, label, px, bw) in [
            (false, "Save CSV…", x, half),
            (true, "Save PNG…", x + half + 6., width - half - 6.),
        ] {
            if f.button_with(
                rect(px, save_y, bw, H),
                label,
                can_save && self.export_job.is_none(),
                Tone::Neutral,
                Some("Save the current Input Pattern"),
            ) {
                self.save_preview(png);
            }
        }
    }

    fn validation_line(&self, f: &mut Form, w: f32) {
        let profile = self.config.profile.generation;
        let text = self.unit_error.clone().map(|e| (e, AMBER)).or_else(|| {
            self.form_spec()
                .and_then(|(_, s)| s.validate(&profile).err())
                .map(|e| (e, ERROR))
        });
        if let Some((text, color)) = text {
            f.note(rect(9., 172., w - 18., 18.), &text, color);
        }
    }

    fn uniform_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
        let max = self.config.profile.generation.max_code;
        let mut f = Form::new(ui, origin);
        f.label(rect(9., 9., 66., H), "Level");
        f.integer(
            "uniform-code",
            rect(81., 9., 88., H),
            &mut self.uniform_code,
            0.,
            f64::from(max),
        );
        f.muted(rect(175., 9., w - 184., H), format!("raw code 0–{max}"));
        self.validation_line(&mut f, w);
    }

    #[allow(clippy::too_many_arguments)]
    fn period_row(
        &mut self,
        f: &mut Form,
        id: &str,
        y: f32,
        x: f32,
        period_of: fn(&mut App) -> &mut PeriodValue,
        w: f32,
        hint: bool,
    ) {
        let optics = self.config.profile.generation.optics;
        let period = period_of(self);
        f.number(
            &format!("{id}-value"),
            rect(x, y, 88., H),
            &mut period.value,
            true,
        );
        let mut unit = period.unit;
        let options: Vec<(PeriodUnit, String, bool)> = PeriodUnit::ALL
            .iter()
            .map(|u| (*u, ui_text(u.suffix()), true))
            .collect();
        f.combo(
            &format!("{id}-unit"),
            rect(x + 94., y, 72., H),
            &mut unit,
            &options,
        );
        let mut error = None;
        if unit != period.unit {
            match period.convert(unit, &optics) {
                Ok(v) => *period = v,
                Err(e) => error = Some(e),
            }
        }
        if hint {
            let text = format!(
                "= {} px · {} µm · {} µm⁻¹",
                period.display_in(PeriodUnit::SlmPx, &optics),
                period.display_in(PeriodUnit::SampleUm, &optics),
                period.display_in(PeriodUnit::SampleWavevector, &optics)
            );
            f.muted(rect(x + 172., y, w - x - 181., H), ui_text(&text));
        }
        if error.is_some() {
            self.unit_error = error;
        }
    }

    fn phase_unit_combo(f: &mut Form, id: &str, r: Rect, phase: &mut PhaseValue) {
        let mut unit = phase.unit;
        let options: Vec<(PhaseUnit, String, bool)> = PhaseUnit::ALL
            .iter()
            .map(|u| (*u, u.suffix().to_string(), true))
            .collect();
        if f.combo(id, r, &mut unit, &options)
            && let Ok(v) = phase.convert(unit)
        {
            *phase = v;
        }
    }

    fn rotation_row(f: &mut Form, id: &str, x: f32, y: f32, geometry: &mut Geometry) {
        f.number(
            &format!("{id}-rot"),
            rect(x, y, 88., H),
            &mut geometry.rotation.value,
            true,
        );
        let mut unit = geometry.rotation.unit;
        let options: Vec<(AngleUnit, String, bool)> = AngleUnit::ALL
            .iter()
            .map(|u| (*u, u.suffix().to_string(), true))
            .collect();
        if f.combo(
            &format!("{id}-rot-unit"),
            rect(x + 94., y, 72., H),
            &mut unit,
            &options,
        ) && let Ok(v) = geometry.rotation.convert(unit)
        {
            geometry.rotation = v;
        }
    }

    fn center_row(f: &mut Form, id: &str, x: f32, y: f32, geometry: &mut Geometry) {
        let [mut cx, mut cy] = geometry.center_offset_px;
        f.integer(
            &format!("{id}-cx"),
            rect(x, y, 60., H),
            &mut cx,
            -100_000.,
            100_000.,
        );
        f.integer(
            &format!("{id}-cy"),
            rect(x + 66., y, 60., H),
            &mut cy,
            -100_000.,
            100_000.,
        );
        geometry.center_offset_px = [cx, cy];
        f.muted(rect(x + 132., y, 24., H), "px");
    }

    fn single_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
        let mut f = Form::new(ui, origin);
        f.label(rect(9., 9., 66., H), "Period");
        self.period_row(
            &mut f,
            "single-period",
            9.,
            81.,
            |a| &mut a.single.wave.period,
            w,
            true,
        );
        f.label(rect(9., 41., 66., H), "Waveform");
        f.segments(
            rect(81., 41., 166., H),
            &mut self.single.wave.waveform,
            &[(Waveform::Ramp, "Ramp"), (Waveform::Binary, "Binary 0/π")],
        );
        f.label(rect(9., 73., 66., H), "Phase");
        f.number(
            "single-phase",
            rect(81., 73., 88., H),
            &mut self.single.wave.initial_phase.value,
            true,
        );
        Self::phase_unit_combo(
            &mut f,
            "single-phase-unit",
            rect(175., 73., 72., H),
            &mut self.single.wave.initial_phase,
        );
        f.label(rect(9., 105., 66., H), "Rotation");
        Self::rotation_row(&mut f, "single", 81., 105., &mut self.single.geometry);
        f.label(rect(9., 137., 66., H), "Center");
        Self::center_row(&mut f, "single", 81., 137., &mut self.single.geometry);
        self.validation_line(&mut f, w);
    }

    /// One row per Multi-Wave component: period + unit, waveform, phase +
    /// unit, remove.
    fn component_rows(
        f: &mut Form,
        components: &mut [WaveComponent],
        optics: &slm_core::Optics,
        remove: &mut Option<usize>,
        unit_error: &mut Option<String>,
    ) {
        let count = components.len();
        for (i, c) in components.iter_mut().enumerate() {
            let y = i as f32 * 30.;
            f.muted(rect(0., y, 22., H), format!("#{}", i + 1));
            f.number(
                &format!("mc-{i}-p"),
                rect(24., y, 72., H),
                &mut c.period.value,
                true,
            );
            let mut unit = c.period.unit;
            let options: Vec<(PeriodUnit, String, bool)> = PeriodUnit::ALL
                .iter()
                .map(|u| (*u, ui_text(u.suffix()), true))
                .collect();
            if f.combo(
                &format!("mc-{i}-u"),
                rect(100., y, 66., H),
                &mut unit,
                &options,
            ) {
                match c.period.convert(unit, optics) {
                    Ok(v) => c.period = v,
                    Err(e) => *unit_error = Some(e),
                }
            }
            f.segments(
                rect(172., y, 130., H),
                &mut c.waveform,
                &[(Waveform::Ramp, "Ramp"), (Waveform::Binary, "0/π")],
            );
            f.label(rect(310., y, 36., H), "Phase");
            f.number(
                &format!("mc-{i}-ph"),
                rect(348., y, 60., H),
                &mut c.initial_phase.value,
                true,
            );
            Self::phase_unit_combo(
                f,
                &format!("mc-{i}-phu"),
                rect(412., y, 52., H),
                &mut c.initial_phase,
            );
            if count > 1
                && f.button_with(
                    rect(470., y, 26., H),
                    "×",
                    true,
                    Tone::Neutral,
                    Some("Remove component"),
                )
            {
                *remove = Some(i);
            }
        }
    }

    fn multi_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
        let optics = self.config.profile.generation.optics;
        let list = rect(origin.x + 9., origin.y + 9., w - 18., 92.);
        let mut remove = None;
        let count = self.multi.components.len();
        let mut unit_error = None;
        // Up to three components fit the reserved rows; only more scroll
        // (floating bar, TRKR style), so no idle scrollbar track is shown.
        if count <= 3 {
            let mut f = Form::new(ui, list.min);
            Self::component_rows(
                &mut f,
                &mut self.multi.components,
                &optics,
                &mut remove,
                &mut unit_error,
            );
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(list));
                ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();
                egui::ScrollArea::vertical()
                    .id_salt("multi-components")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let top = ui.max_rect().min;
                        let mut f = Form::new(ui, top);
                        Self::component_rows(
                            &mut f,
                            &mut self.multi.components,
                            &optics,
                            &mut remove,
                            &mut unit_error,
                        );
                        f.ui.set_min_size(Vec2::new(500., count as f32 * 30.));
                    });
            });
        }
        if let Some(i) = remove {
            self.multi.components.remove(i);
        }
        if unit_error.is_some() {
            self.unit_error = unit_error;
        }
        let default_band = self.config.profile.generation.default_stripe_width_px;
        let mut f = Form::new(ui, origin);
        if f.button(rect(9., 105., 66., H), "+ Wave", true) {
            let last = *self
                .multi
                .components
                .last()
                .expect("at least one component");
            self.multi.components.push(last);
        }
        f.label(rect(87., 105., 40., H), "Band");
        if !self.multi.band_override {
            self.multi.band_width = default_band;
        }
        f.enabled = self.multi.band_override;
        f.integer(
            "multi-band",
            rect(127., 105., 50., H),
            &mut self.multi.band_width,
            1.,
            10_000.,
        );
        f.enabled = true;
        f.check(
            rect(183., 105., 84., H),
            &mut self.multi.band_override,
            "Override",
        );
        f.check(
            rect(271., 105., 160., H),
            &mut self.multi.reverse_band_order,
            "Reverse Band Order",
        );
        f.label(rect(9., 137., 66., H), "Rotation");
        Self::rotation_row(&mut f, "multi", 81., 137., &mut self.multi.geometry);
        f.label(rect(259., 137., 46., H), "Center");
        Self::center_row(&mut f, "multi", 307., 137., &mut self.multi.geometry);
        self.validation_line(&mut f, w);
    }

    fn sweep_targets(&self) -> Vec<SweepTarget> {
        match self.sweep.base {
            SweepBase::Uniform => vec![SweepTarget::UniformCode],
            SweepBase::Single => vec![
                SweepTarget::Period { component: 0 },
                SweepTarget::InitialPhase { component: 0 },
                SweepTarget::Rotation,
            ],
            SweepBase::Multi => {
                let mut t = Vec::new();
                for component in 0..self.multi.components.len() {
                    t.push(SweepTarget::Period { component });
                    t.push(SweepTarget::InitialPhase { component });
                }
                t.push(SweepTarget::Rotation);
                t
            }
        }
    }

    fn sweep_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
        let mut f = Form::new(ui, origin);
        f.label(rect(9., 9., 66., H), "Base");
        let before = self.sweep.base;
        f.segments(
            rect(81., 9., 260., H),
            &mut self.sweep.base,
            &[
                (SweepBase::Uniform, "Uniform"),
                (SweepBase::Single, "Single Wave"),
                (SweepBase::Multi, "Multi-Wave"),
            ],
        );
        let targets = self.sweep_targets();
        if before != self.sweep.base || !targets.contains(&self.sweep.target) {
            self.sweep.target = targets[0];
            if let Some(v) = self.sweep.target.value(&self.sweep_spec().base) {
                self.sweep.start = v;
                self.sweep.end = v;
            }
        }
        let base = self.sweep_spec().base;
        f.label(rect(9., 41., 66., H), "Quantity");
        let options: Vec<(SweepTarget, String, bool)> = targets
            .iter()
            .map(|t| (*t, ui_text(&t.label(&base)), true))
            .collect();
        f.combo(
            "sweep-target",
            rect(81., 41., 260., H),
            &mut self.sweep.target,
            &options,
        );
        let unit = ui_text(&self.sweep.target.unit(&base).unwrap_or_default());
        f.label(rect(9., 73., 66., H), "Start");
        f.number(
            "sweep-start",
            rect(81., 73., 88., H),
            &mut self.sweep.start,
            true,
        );
        f.label(rect(181., 73., 40., H), "End");
        f.number(
            "sweep-end",
            rect(221., 73., 88., H),
            &mut self.sweep.end,
            true,
        );
        f.muted(rect(315., 73., 60., H), unit.clone());
        f.label(rect(9., 105., 66., H), "Step");
        f.number(
            "sweep-step",
            rect(81., 105., 88., H),
            &mut self.sweep.step,
            true,
        );
        f.muted(rect(175., 105., 40., H), unit.clone());
        f.check(
            rect(221., 105., 200., H),
            &mut self.sweep.include_end,
            "Append end if off grid",
        );
        let spec = self.sweep_spec();
        match spec.summary() {
            Ok(p) => {
                let invalid = self.sweep_validation().err();
                let text = format!(
                    "Last {} {} · {} {}",
                    format_number(p.last),
                    unit,
                    p.count,
                    if p.count == 1 { "point" } else { "points" },
                );
                f.note(
                    rect(375., 73., w - 384., H),
                    &text,
                    if p.end_off_grid { AMBER } else { TEXT },
                );
                let grid_note = match (p.end_off_grid, self.sweep.include_end) {
                    (true, true) => "Off-grid end appended",
                    (true, false) => "Off-grid end omitted",
                    _ => "",
                };
                if !grid_note.is_empty() {
                    f.note(rect(221., 134., w - 230., 20.), grid_note, AMBER);
                }
                if let Some(e) = invalid {
                    f.note(rect(9., 172., w - 18., 18.), &e, ERROR);
                }
            }
            Err(e) => {
                f.note(rect(9., 140., w - 18., 20.), &e, ERROR);
            }
        }
    }

    fn pick_save_path(&self) -> Option<PathBuf> {
        self.file_dialog("Save list")
            .add_filter("KohdaLab SLM experiment", &["json"])
            .set_file_name(format!("{}.json", self.experiment.experiment.name))
            .save_file()
    }

    fn experiment_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
        let path = self
            .experiment
            .path
            .as_ref()
            .map_or_else(|| "Not saved".to_string(), |p| p.display().to_string());
        let mut f = Form::new(ui, origin);
        f.label(rect(9., 9., 66., H), "Name");
        if f.text(
            "experiment-name",
            rect(81., 9., 200., H),
            &mut self.experiment.experiment.name,
        )
        .changed()
        {
            self.experiment.dirty = true;
        }
        if self.experiment.dirty {
            f.value(rect(289., 9., 80., H), "unsaved", AMBER);
        }
        f.label(rect(9., 41., 66., H), "File");
        f.path_summary(rect(81., 41., w - 90., H), &path);
        let bw = 96.;
        let buttons = ["New List", "Open List…", "Save List", "Save List As…"];
        let mut clicked = None;
        for (i, name) in buttons.iter().enumerate() {
            if f.button(rect(81. + i as f32 * (bw + 6.), 73., bw, H), name, true) {
                clicked = Some(*name);
            }
        }
        let import = f.button(rect(81., 105., bw, H), "Import CSV…", true);
        let export = f.button(
            rect(81. + bw + 6., 105., bw, H),
            "Export…",
            !self.experiment.experiment.items.is_empty() && self.export_job.is_none(),
        );
        f.label(rect(9., 137., 66., H), "PNG");
        let mut x = 81.;
        for kind in HeatmapKind::ALL {
            let mut on = self.heatmaps.contains(&kind);
            let width = match kind {
                HeatmapKind::Kerr => 110.,
                _ => 90.,
            };
            if f.check(rect(x, 137., width, H), &mut on, kind.label()) {
                if on {
                    self.heatmaps.insert(kind);
                } else {
                    self.heatmaps.remove(&kind);
                }
            }
            x += width + 6.;
        }
        if let Some((done, total)) = self.export_progress {
            f.value(
                rect(81. + 2. * (bw + 6.), 105., 140., H),
                if self.export_job.as_ref().is_some_and(|j| j.cancellable) {
                    format!("Exporting {done}/{total}")
                } else {
                    "Saving file…".into()
                },
                TEAL,
            );
            if f.button(
                rect(81. + 2. * (bw + 6.) + 144., 105., 110., H),
                "Cancel Export",
                self.export_job.as_ref().is_some_and(|j| j.cancellable),
            ) && let Some(job) = &self.export_job
            {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let mut discard = None;
        if let Some(action) = self.confirm_discard {
            f.value(
                rect(9., 169., 240., H),
                "Unsaved list changes will be lost.",
                AMBER,
            );
            if f.button(rect(255., 169., 140., H), "Discard and Continue", true) {
                discard = Some(action);
            }
            if f.button(rect(401., 169., 70., H), "Cancel", true) {
                self.confirm_discard = None;
            }
        } else if let Some(item) = self
            .selected
            .and_then(|i| self.experiment.experiment.items.get(i))
        {
            let text = ui_text(&format!(
                "Row #{}: {}",
                self.selected.unwrap_or(0) + 1,
                item.summary()
            ));
            f.note(rect(9., 169., w - 18., 20.), &text, MUTED);
        }
        match clicked {
            Some("New List") if self.experiment.dirty => self.confirm_discard = Some(Discard::New),
            Some("New List") => {
                self.experiment = ExperimentDoc::new("Untitled");
                self.set_selection(None);
                self.reveal_row = None;
            }
            Some("Open List…") if self.experiment.dirty => {
                self.confirm_discard = Some(Discard::Open)
            }
            Some("Open List…") => {
                if let Some(p) = self
                    .file_dialog("Open list")
                    .add_filter("KohdaLab SLM experiment", &["json"])
                    .pick_file()
                {
                    self.open_experiment(&p);
                }
            }
            Some("Save List") => {
                if let Some(p) = self
                    .experiment
                    .path
                    .clone()
                    .or_else(|| self.pick_save_path())
                {
                    self.save_experiment(&p);
                }
            }
            Some(_) => {
                if let Some(p) = self.pick_save_path() {
                    self.save_experiment(&p);
                }
            }
            None => {}
        }
        if let Some(action) = discard {
            self.confirm_discard = None;
            match action {
                Discard::New => {
                    self.experiment = ExperimentDoc::new("Untitled");
                    self.set_selection(None);
                    self.reveal_row = None;
                }
                Discard::Open => {
                    if let Some(p) = self
                        .file_dialog("Open list")
                        .add_filter("KohdaLab SLM experiment", &["json"])
                        .pick_file()
                    {
                        self.open_experiment(&p);
                    }
                }
            }
        }
        if import
            && let Some(files) = self
                .file_dialog("Import CSV")
                .add_filter("Santec CSV", &["csv"])
                .pick_files()
        {
            for file in files {
                self.import_csv(&file);
            }
        }
        if export && let Some(dir) = self.file_dialog("Export folder").pick_folder() {
            self.dialog_dirs.insert("Export folder".into(), dir.clone());
            self.export_experiment(&dir.join(format!("slm-export-{}", new_id())));
        }
    }

    // ----- shared list ---------------------------------------------------

    fn pattern_label(item: &ExperimentItem) -> String {
        item.sweep.as_ref().map_or_else(
            || {
                if matches!(item.label.as_str(), "Single Wave" | "Multi-Wave") {
                    format!("{} · {}", item.label, item.summary())
                } else {
                    item.label.clone()
                }
            },
            |sweep| {
                let unit = ui_text(&sweep.unit);
                format!(
                    "{} = {} {unit}",
                    sweep.target_label,
                    format_number(sweep.value)
                )
                .trim_end()
                .into()
            },
        )
    }

    fn row_caption(&self, index: usize) -> String {
        let item = &self.experiment.experiment.items[index];
        format!("Row #{} · {}", index + 1, Self::pattern_label(item))
    }

    fn preview_status(&self) -> [(String, Color32); 3] {
        let selected = self.selected.map_or_else(
            || "Selected · none".into(),
            |index| format!("Selected · {}", self.row_caption(index)),
        );
        let in_progress = if let Some((_, record)) = &self.pending {
            let mut text = format!(
                "Applying · {}",
                self.output_caption(record.item_id.as_deref(), &record.label)
            );
            if let Some(queued) = self.queued_row {
                text.push_str(&format!(" · Queued #{}", queued + 1));
            }
            (text, AMBER)
        } else if let ApplyStatus::Failed { record, .. } = &self.status {
            (format!("Failed · {}", record.label), ERROR)
        } else {
            ("Applying · none".into(), MUTED)
        };
        let applied = match &self.device_state {
            DeviceState::Connected { showing, .. } => (
                format!(
                    "Applied · {}",
                    self.output_caption(showing.item_id.as_deref(), &showing.label)
                ),
                TEAL,
            ),
            DeviceState::Unknown { .. } => ("Output unknown".into(), ERROR),
            _ => ("Applied · none".into(), MUTED),
        };
        [(selected, TEXT), in_progress, applied]
    }

    fn output_caption(&self, item_id: Option<&str>, fallback: &str) -> String {
        item_id
            .and_then(|id| {
                self.experiment
                    .experiment
                    .items
                    .iter()
                    .position(|i| i.id == id)
            })
            .map_or_else(|| fallback.to_string(), |index| self.row_caption(index))
    }

    /// Shared list for selecting, applying and editing patterns.
    fn list_card(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32, height: f32) {
        let n = self.experiment.experiment.items.len();
        let showing = self.showing_index();
        let unknown = matches!(self.device_state, DeviceState::Unknown { .. });
        let last_id = match &self.status {
            ApplyStatus::Applied(r, _) | ApplyStatus::Applying(r) => r.item_id.as_deref(),
            ApplyStatus::Failed { record, .. } => record.item_id.as_deref(),
            ApplyStatus::Idle => None,
        };
        let pending_id = self
            .pending
            .as_ref()
            .and_then(|(_, r)| r.item_id.as_deref());
        let selected_count = self.selected_rows.len();
        let previous = self.adjacent_row(false);
        let next = self.adjacent_row(true);
        let block = self.apply_block();
        let mut apply = None;
        let remove;
        let mut undo = false;
        {
            let mut f = Form::new(ui, origin);
            f.group(rect(0., 0., w, height), "Patterns");
            let stats = format!("{n} items · {selected_count} selected");
            f.note(rect(w - 175., 4., 164., 18.), &stats, MUTED);
            let action_width = 108.;
            let action_x = w - 11. - action_width;
            for (y, label, index, direction, hint) in [
                (
                    27.,
                    "Previous",
                    previous,
                    -1.,
                    "Apply the preceding list row",
                ),
                (55., "Next", next, 1., "Apply the following list row"),
            ] {
                let enabled = block.is_none() && index.is_some();
                if f.button_with(
                    rect(action_x, y, action_width, 24.),
                    label,
                    enabled,
                    Tone::Neutral,
                    Some(block.unwrap_or(hint)),
                ) {
                    apply = index;
                }
                let center = origin + Vec2::new(action_x + 15., y + 12.);
                let tip = center + Vec2::new(0., direction * 4.);
                let stroke = egui::Stroke::new(1.3_f32, if enabled { TEXT } else { MUTED });
                f.ui.painter()
                    .line_segment([center - Vec2::new(0., direction * 4.), tip], stroke);
                for x in [-3.5, 3.5] {
                    f.ui.painter()
                        .line_segment([tip, tip + Vec2::new(x, -direction * 3.5)], stroke);
                }
            }
            let main_width = action_x - 17.;
            if f.button_with(
                rect(11., 27., main_width, 24.),
                "Apply Row",
                block.is_none() && self.selected.is_some(),
                Tone::Primary,
                Some(block.unwrap_or("Apply the focused list row · Ctrl/Cmd+Enter")),
            ) {
                apply = self.selected;
            }
            if f.check(
                rect(11., 55., main_width.min(154.), 24.),
                &mut self.auto_apply_rows,
                "Auto-apply on click",
            ) {
                self.queued_row = None;
            }

            let footer = height - 34.;
            let half = (w - 34.) / 3.;
            remove = f.button_with(
                rect(11., footer, half, 24.),
                "Remove Selected",
                selected_count > 0,
                Tone::Neutral,
                Some("Remove selected rows from the list"),
            );
            if f.button(
                rect(17. + half, footer, half, 24.),
                "Undo Remove",
                self.undo_removed
                    .as_ref()
                    .is_some_and(|(id, _)| id == &self.experiment.experiment.id),
            ) {
                undo = true;
            }
            if f.button(
                rect(23. + 2. * half, footer, half, 24.),
                "Remove All…",
                n > 0,
            ) {
                self.confirm_remove_all = true;
            }
        }
        let table = rect(origin.x + 11., origin.y + 87., w - 22., height - 129.);
        ui.painter().rect(
            table,
            0.,
            theme::BACKGROUND,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let header = rect(table.min.x + 1., table.min.y + 1., table.width() - 2., 22.);
        ui.painter().rect_filled(header, 0., HEADER_FILL);
        let (num_w, state_w) = (34., 74.);
        for (title, x) in [
            ("#", header.min.x),
            ("Label", header.min.x + num_w),
            ("State", header.max.x - state_w),
        ] {
            ui.painter().text(
                Pos2::new(x + 6., header.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(12.),
                TEXT,
            );
        }
        let body = Rect::from_min_max(
            Pos2::new(table.min.x + 1., header.max.y),
            table.max - Vec2::new(1., 1.),
        );
        let keyboard = ui.interact(
            body,
            ui.id().with("patterns-keyboard"),
            egui::Sense::focusable_noninteractive(),
        );
        let lock_arrows = |memory: &mut egui::Memory| {
            memory.set_focus_lock_filter(
                keyboard.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        };
        let key_step = if keyboard.has_focus() && !self.confirm_remove_all {
            ui.memory_mut(lock_arrows);
            ui.painter().rect_stroke(
                table,
                0.,
                egui::Stroke::new(1_f32, TEAL),
                egui::StrokeKind::Inside,
            );
            ui.input_mut(|input| {
                [(egui::Key::ArrowUp, false), (egui::Key::ArrowDown, true)]
                    .into_iter()
                    .find_map(|(key, forward)| {
                        input
                            .consume_key(egui::Modifiers::NONE, key)
                            .then_some(forward)
                    })
            })
        } else {
            None
        };
        let (mut clicked, mut drop) = (None, None);
        const ROW_HEIGHT: f32 = 24.;
        let reveal = self.reveal_row.take();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(body));
                ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();
                ui.spacing_mut().item_spacing.y = 0.;
                if n == 0 {
                    ui.add_space(8.);
                    ui.label(RichText::new("Add to List, Add Sweep or Import CSV…").color(MUTED));
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("pattern-list")
                    .max_height(body.height())
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, n, |ui, rows| {
                        let width = ui.available_width();
                        if let Some(index) = reveal.filter(|&i| i < n) {
                            let y = ui.max_rect().top()
                                + (index as f32 - rows.start as f32) * ROW_HEIGHT;
                            ui.scroll_to_rect(
                                rect(ui.max_rect().left(), y, width, ROW_HEIGHT),
                                None,
                            );
                        }
                        let pointer = ui.input(|i| i.pointer.latest_pos());
                        if egui::DragAndDrop::has_payload_of_type::<String>(ui.ctx())
                            && let Some(pos) = pointer.filter(|p| body.contains(*p))
                        {
                            let delta = (body.top() + 18. - pos.y).max(0.)
                                - (pos.y - body.bottom() + 18.).max(0.);
                            if delta != 0. {
                                ui.scroll_with_delta_animation(
                                    Vec2::new(0., delta * 0.6),
                                    egui::style::ScrollAnimation::none(),
                                );
                                ui.ctx().request_repaint();
                            }
                        }
                        for i in rows {
                            let item = &self.experiment.experiment.items[i];
                            let (row, response) = ui.allocate_exact_size(
                                Vec2::new(width, ROW_HEIGHT),
                                egui::Sense::click_and_drag(),
                            );
                            let selected = self.selected_rows.contains(&i);
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    true,
                                    selected,
                                    format!("#{} {}", i + 1, Self::pattern_label(item)),
                                )
                            });
                            if response.drag_started() {
                                response.dnd_set_drag_payload(item.id.clone());
                            }
                            let fill = if selected {
                                theme::SELECTION
                            } else if response.hovered() {
                                Color32::from_rgb(34, 35, 41)
                            } else if i % 2 == 1 {
                                Color32::from_rgb(22, 23, 28)
                            } else {
                                theme::BACKGROUND
                            };
                            ui.painter().rect_filled(row, 0., fill);
                            if selected && self.selected == Some(i) {
                                ui.painter().rect_filled(
                                    rect(row.left(), row.top(), 2., row.height()),
                                    0.,
                                    TEAL,
                                );
                            }
                            let (state, color) = if pending_id == Some(item.id.as_str()) {
                                ("Applying…", AMBER)
                            } else if showing == Some(i) {
                                ("Applied", TEAL)
                            } else if unknown && last_id == Some(item.id.as_str()) {
                                ("Unknown", ERROR)
                            } else if self.queued_row == Some(i) {
                                ("Queued", AMBER)
                            } else if selected {
                                ("Selected", TEXT)
                            } else {
                                ("", MUTED)
                            };
                            let cells = [
                                (format!("{}", i + 1), row.min.x, num_w, MUTED),
                                (
                                    Self::pattern_label(item),
                                    row.min.x + num_w,
                                    row.width() - num_w - state_w,
                                    TEXT,
                                ),
                                (state.to_string(), row.max.x - state_w, state_w, color),
                            ];
                            for (text, x, cw, color) in cells {
                                let cell =
                                    rect(x + 6., row.min.y, (cw - 10.).max(0.), row.height());
                                let mut job = egui::text::LayoutJob::simple_singleline(
                                    text,
                                    egui::FontId::proportional(12.),
                                    color,
                                );
                                job.wrap =
                                    egui::text::TextWrapping::truncate_at_width(cell.width());
                                let galley = ui.painter().layout_job(job);
                                ui.painter()
                                    .with_clip_rect(cell.intersect(ui.clip_rect()))
                                    .galley(
                                        Pos2::new(
                                            cell.min.x,
                                            cell.center().y - galley.size().y * 0.5,
                                        ),
                                        galley,
                                        color,
                                    );
                            }
                            if response.dnd_hover_payload::<String>().is_some()
                                && let Some(pos) = pointer
                            {
                                let after = pos.y >= row.center().y;
                                let y = if after { row.bottom() } else { row.top() };
                                ui.painter().line_segment(
                                    [Pos2::new(row.left(), y), Pos2::new(row.right(), y)],
                                    egui::Stroke::new(2_f32, TEAL),
                                );
                                if let Some(id) = response.dnd_release_payload::<String>() {
                                    drop = Some((id, i + usize::from(after)));
                                }
                            }
                            let response = response.on_hover_text(format!(
                                "#{} {} · {}\n{}\n{}",
                                i + 1,
                                item.kind_label(),
                                item.label,
                                ui_text(&item.summary()),
                                item.id
                            ));
                            if response.clicked() {
                                clicked = Some((i, ui.input(|input| input.modifiers)));
                            }
                        }
                    });
            },
        );
        if let Some((index, modifiers)) = clicked {
            self.select_row(index, modifiers);
        }
        if let Some(index) = key_step.and_then(|forward| self.adjacent_row(forward)) {
            self.select_row(index, egui::Modifiers::NONE);
            self.reveal_row = Some(index);
        }
        if let Some((id, gap)) = drop {
            self.move_rows(&id, gap);
        }
        if let Some(index) = apply {
            self.apply_experiment_item(index);
        }
        if clicked.is_some() || apply.is_some() {
            keyboard.request_focus();
            ui.memory_mut(lock_arrows);
        }
        if undo {
            self.undo_remove();
        }
        if remove {
            self.remove_rows(false);
        }
        if self.confirm_remove_all {
            let modal =
                egui::Modal::new(egui::Id::new("remove-all-patterns")).show(ui.ctx(), |ui| {
                    ui.heading("Remove all patterns?");
                    ui.label(format!("Remove all {n} rows from this list."));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            ui.close();
                        }
                        ui.button("Remove All Patterns").clicked()
                    })
                    .inner
                });
            if modal.inner {
                self.remove_rows(true);
            }
            if modal.inner || modal.should_close() {
                self.confirm_remove_all = false;
            }
        }
    }

    /// Lower center: the SLM code heatmap of the current input, as TRKR's
    /// plot area. Image +X is right (column 0 left), +Y is down (row 0 top),
    /// square SLM pixels, fixed color scale 0 … profile max code in the
    /// selected colormap, with a matching colorbar on the right.
    fn input_pattern(&mut self, ui: &mut egui::Ui, area: Rect) {
        let current = self.current_preview();
        let mut use_row = self.sweep_preview_row;
        {
            let mut f = Form::new(ui, area.min);
            f.label(rect(0., 0., 92., 24.), "Input Pattern");
            let caption = current
                .as_ref()
                .map_or_else(|| "No list row selected".to_string(), |(_, _, c)| c.clone());
            let mut caption_w = area.width() - 98.;
            // Compact colormap selector at the right end of the header.
            const MAP_W: f32 = 104.;
            caption_w -= MAP_W + 6.;
            let mut colormap = self.config.preview_colormap;
            let options: Vec<(Colormap, String, bool)> = Colormap::ALL
                .into_iter()
                .map(|m| (m, m.label().to_string(), true))
                .collect();
            if f.combo(
                "preview-colormap",
                rect(area.width() - MAP_W, 0., MAP_W, 24.),
                &mut colormap,
                &options,
            ) {
                self.config.preview_colormap = colormap;
                self.config_dirty = true;
            }
            if self.tab == Tab::Sweep {
                caption_w -= 186.;
                let x = 98. + caption_w + 6.;
                f.enabled = self.selected.is_some() || use_row;
                f.segments(
                    rect(x, 0., 180., 24.),
                    &mut use_row,
                    &[(false, "First value"), (true, "List row")],
                );
                f.enabled = true;
            }
            f.note(rect(98., 0., caption_w, 24.), &caption, TEXT);
            let third = area.width() / 3.;
            for (slot, (status, color)) in self.preview_status().into_iter().enumerate() {
                f.note(
                    rect(slot as f32 * third, 26., third - 8., 22.),
                    &status,
                    color,
                );
            }
        }
        self.sweep_preview_row = use_row;
        let frame = Rect::from_min_max(area.min + Vec2::new(0., 54.), area.max);
        ui.painter().rect(
            frame,
            0.,
            theme::BACKGROUND,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let font = egui::FontId::proportional(12.);
        let painter = ui.painter().with_clip_rect(frame);
        let key = current.as_ref().map(|(k, _, _)| k.clone());
        let shown = key.is_some() && self.preview.shown == key;
        let message = |text: &str, color: Color32| {
            painter.text(
                frame.center(),
                egui::Align2::CENTER_CENTER,
                text,
                font.clone(),
                color,
            );
        };
        let Some(key) = key else {
            message(
                "Select a row in the Patterns list to preview it (preview only; nothing is sent).",
                MUTED,
            );
            return;
        };
        if !shown {
            match &self.preview.error {
                Some((k, e)) if *k == key => message(&format!("Cannot render: {e}"), ERROR),
                _ => message("Rendering input pattern…", MUTED),
            }
            return;
        }
        let (Some(texture), Some(info)) = (&self.preview.texture, &self.preview.info) else {
            return;
        };
        // Fit the SLM raster with square pixels, centered in the frame with
        // room for tick labels on the left and below.
        let (left, right, top, bottom) = (72., 78., 22., 46.);
        let avail = Vec2::new(frame.width() - left - right, frame.height() - top - bottom);
        let aspect = info.width as f32 / info.height.max(1) as f32;
        let (mut iw, mut ih) = (avail.x, avail.x / aspect);
        if ih > avail.y {
            ih = avail.y;
            iw = ih * aspect;
        }
        let x = (frame.center().x - iw * 0.5)
            .max(frame.min.x + left)
            .min(frame.max.x - right - iw);
        let y = frame.min.y + top + (avail.y - ih) * 0.5;
        let image = Rect::from_min_size(Pos2::new(x, y), Vec2::new(iw, ih));
        painter.image(
            texture.id(),
            image,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
            Color32::WHITE,
        );
        painter.rect_stroke(
            image,
            0.,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Outside,
        );
        let axes = info.axes;
        let unit = axes.unit.label();
        let small = egui::FontId::proportional(11.);
        let tick_stroke = egui::Stroke::new(1_f32, MUTED);
        self.colorbar(&painter, image, info, &small, tick_stroke);
        // Screen position of a coordinate: pixel centers, +X right, +Y down,
        // origin at the exact image center.
        let sx = |c: f64| image.min.x + axes.fraction(0, c) as f32 * image.width();
        let sy = |c: f64| image.min.y + axes.fraction(1, c) as f32 * image.height();
        let (x0, x1) = axes.range(0);
        let (xticks, xstep) =
            crate::preview::nice_ticks(x0, x1, ((image.width() / 60.) as usize).max(3));
        for t in xticks {
            let x = sx(t);
            painter.line_segment(
                [Pos2::new(x, image.max.y), Pos2::new(x, image.max.y + 5.)],
                tick_stroke,
            );
            painter.text(
                Pos2::new(x, image.max.y + 7.),
                egui::Align2::CENTER_TOP,
                crate::preview::tick_label(t, xstep),
                small.clone(),
                if t == 0. { TEXT } else { MUTED },
            );
        }
        let (y0, y1) = axes.range(1);
        let (yticks, ystep) =
            crate::preview::nice_ticks(y0, y1, ((image.height() / 55.) as usize).max(3));
        // Leftmost edge of the y tick labels, for the rotated axis title.
        let mut labels_left = image.min.x - 8.;
        for t in yticks {
            let y = sy(t);
            painter.line_segment(
                [Pos2::new(image.min.x - 5., y), Pos2::new(image.min.x, y)],
                tick_stroke,
            );
            let label = painter.text(
                Pos2::new(image.min.x - 8., y),
                egui::Align2::RIGHT_CENTER,
                crate::preview::tick_label(t, ystep),
                small.clone(),
                if t == 0. { TEXT } else { MUTED },
            );
            labels_left = labels_left.min(label.min.x);
        }
        painter.text(
            Pos2::new(image.center().x, image.max.y + 24.),
            egui::Align2::CENTER_TOP,
            format!("x ({unit})"),
            small.clone(),
            TEXT,
        );
        // y title: rotated 90° counterclockwise about the text's own center,
        // left of the tick labels and vertically centered on the plot, kept
        // inside the frame's left margin.
        let galley = painter.layout_no_wrap(format!("y ({unit})"), small.clone(), TEXT);
        let thickness = galley.size().y;
        let center = Pos2::new(
            (labels_left - 6. - thickness * 0.5).max(frame.min.x + thickness * 0.5 + 2.),
            image.center().y,
        );
        let top_left = center - galley.rect.center().to_vec2();
        painter.add(
            egui::epaint::TextShape::new(top_left, galley, TEXT)
                .with_angle_and_anchor(-std::f32::consts::FRAC_PI_2, egui::Align2::CENTER_CENTER),
        );
        // Open reticle at the image center: four short arms around a small
        // gap, fixed logical size, white over a dark edge. Drawn over the
        // image only; never part of the data.
        let center = Pos2::new(sx(0.), sy(0.));
        let (gap, arm) = (4_f32, 7_f32);
        let reticle = image.intersect(Rect::from_center_size(
            center,
            Vec2::splat(2. * (gap + arm) + 4.),
        ));
        let reticle_painter = painter.with_clip_rect(reticle);
        for (width, color) in [
            (3.5_f32, Color32::from_black_alpha(200)),
            (1.5_f32, Color32::WHITE),
        ] {
            let s = egui::Stroke::new(width, color);
            for d in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                reticle_painter.line_segment([center + d * gap, center + d * (gap + arm)], s);
            }
        }
        ui.interact(image, egui::Id::new("input-pattern-hover"), egui::Sense::hover())
            .on_hover_text(format!(
                "{} × {} px · origin (0, 0) at the image center, pixel ({:.1}, {:.1}) · {} per px\ncodes in use {}–{}, color scale 0–{} (profile max), {}\nframe sha256 {}…\nPreview only: identical to what Apply would send, but nothing is sent.",
                info.width,
                info.height,
                axes.origin_px[0],
                axes.origin_px[1],
                if axes.unit == crate::preview::AxisUnit::Px {
                    "1 px".to_string()
                } else {
                    format!("{} µm", format_number(axes.unit_per_px))
                },
                info.min_code,
                info.max_code_used,
                info.scale_max,
                info.colormap.label(),
                info.digest.get(..16).unwrap_or(&info.digest),
            ));
    }

    /// Vertical colorbar right of the image: code 0 at the bottom, the
    /// profile max code at the top, in the colormap the image was drawn with.
    /// Uses the same `Colormap::rgb(code / max)` as the image pixels.
    fn colorbar(
        &self,
        painter: &egui::Painter,
        image: Rect,
        info: &crate::preview::PreviewImage,
        font: &egui::FontId,
        tick_stroke: egui::Stroke,
    ) {
        let bar = Rect::from_min_max(
            Pos2::new(image.max.x + 12., image.min.y),
            Pos2::new(image.max.x + 24., image.max.y),
        );
        let max = f64::from(info.scale_max.max(1));
        let strips = (bar.height().round() as usize).clamp(1, 256);
        let strip_h = bar.height() / strips as f32;
        for i in 0..strips {
            // Strip i covers codes from the top down; color at its center.
            let t = 1. - (i as f64 + 0.5) / strips as f64;
            let [r, g, b] = info.colormap.rgb(t);
            let y = bar.min.y + i as f32 * strip_h;
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(bar.min.x, y),
                    Pos2::new(bar.max.x, y + strip_h + 0.5),
                ),
                0.,
                Color32::from_rgb(r, g, b),
            );
        }
        painter.rect_stroke(
            bar,
            0.,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Outside,
        );
        let (ticks, step) = crate::preview::nice_ticks(0., max, 5);
        let mut values: Vec<f64> = ticks
            .into_iter()
            .filter(|t| *t < max - step * 0.35)
            .collect();
        values.push(max);
        for v in values {
            let y = bar.max.y - (v / max) as f32 * bar.height();
            painter.line_segment(
                [Pos2::new(bar.max.x, y), Pos2::new(bar.max.x + 4., y)],
                tick_stroke,
            );
            painter.text(
                Pos2::new(bar.max.x + 6., y),
                egui::Align2::LEFT_CENTER,
                crate::preview::tick_label(v, 1.),
                font.clone(),
                MUTED,
            );
        }
        painter.text(
            Pos2::new(bar.center().x, bar.max.y + 7.),
            egui::Align2::CENTER_TOP,
            "code",
            font.clone(),
            TEXT,
        );
    }

    // ----- right rail ----------------------------------------------------

    fn log_view(&mut self, ui: &mut egui::Ui, area: Rect) {
        ui.painter().rect(
            area,
            8.,
            theme::BACKGROUND,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let inner = area.shrink(4.);
        // Status line and log body are separate rectangles; the body is
        // clipped to its own rect so scrolled lines never cover the status.
        let simulated = self.config.output.backend == BackendKind::Demo;
        let status_h = if simulated { 20. } else { 0. };
        if simulated {
            let status = rect(inner.min.x + 2., inner.min.y, inner.width() - 4., status_h);
            ui.painter().with_clip_rect(status).text(
                Pos2::new(status.min.x, status.center().y),
                egui::Align2::LEFT_CENTER,
                "DEMO — simulated output · no hardware",
                egui::FontId::proportional(12.),
                TEXT,
            );
            ui.painter().line_segment(
                [
                    Pos2::new(inner.min.x, inner.min.y + status_h + 2.),
                    Pos2::new(inner.max.x, inner.min.y + status_h + 2.),
                ],
                egui::Stroke::new(1_f32, BORDER),
            );
        }
        let body = Rect::from_min_max(
            Pos2::new(
                inner.min.x,
                inner.min.y + status_h + (if self.demo { 6. } else { 0. }),
            ),
            inner.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(body));
                ui.spacing_mut().item_spacing.y = 2.;
                egui::ScrollArea::vertical()
                    .id_salt("log-scroll")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.log {
                            let color = match line.level {
                                Level::Info => TEXT,
                                Level::Warn => AMBER,
                                Level::Error => ERROR,
                            };
                            ui.add(egui::Label::new(RichText::new(&line.text).color(color)).wrap())
                                .on_hover_text(&line.time);
                        }
                    });
            },
        );
    }

    fn applied_fields(&self) -> Vec<(&'static str, String)> {
        let dash = || "-".to_string();
        let (state, _) = self.state_caption();
        let mut rows = vec![
            (
                "state",
                match &self.device_state {
                    DeviceState::Disconnected => "disconnected".into(),
                    DeviceState::Connecting => "connecting".into(),
                    DeviceState::Connected { .. } => "connected".into(),
                    DeviceState::Unknown { .. } => "output unknown".into(),
                },
            ),
            ("backend", self.config.output.backend.label().into()),
            ("output", if state == "----" { dash() } else { state }),
            (
                "mode",
                self.config
                    .confirmed_mode()
                    .map_or_else(|| "not confirmed".into(), |m| m.short().into()),
            ),
        ];
        let (record, receipt, error) = match &self.status {
            ApplyStatus::Applied(r, receipt) => (Some(r), Some(receipt), None),
            ApplyStatus::Applying(r) => (Some(r), None, None),
            ApplyStatus::Failed { record, error, .. } => (Some(record), None, Some(error.clone())),
            ApplyStatus::Idle => (None, None, None),
        };
        rows.push((
            "status",
            match &self.status {
                ApplyStatus::Idle => dash(),
                ApplyStatus::Applying(_) => "applying".into(),
                ApplyStatus::Applied(..) => "applied".into(),
                ApplyStatus::Failed {
                    output_known: true, ..
                } => "not applied".into(),
                ApplyStatus::Failed { .. } => "failed, output unknown".into(),
            },
        ));
        rows.push(("label", record.map_or_else(dash, |r| r.label.clone())));
        rows.push((
            "item_id",
            record.and_then(|r| r.item_id.clone()).unwrap_or_else(dash),
        ));
        match record.and_then(|r| r.spec.as_ref()) {
            Some(PatternSpec::Uniform { code }) => {
                rows.push(("kind", "uniform".into()));
                rows.push(("code", code.to_string()));
            }
            Some(spec @ (PatternSpec::SingleWave { .. } | PatternSpec::MultiWave { .. })) => {
                rows.push(("kind", spec.kind_label().into()));
                for (i, c) in spec.components().iter().enumerate() {
                    let n = spec.components().len() > 1;
                    let key = |s: &'static str, k: &'static str| if n { k } else { s };
                    let period = format!(
                        "{} {}",
                        format_number(c.period.value),
                        ui_text(c.period.unit.suffix())
                    );
                    let phase = format!(
                        "{} {}",
                        format_number(c.initial_phase.value),
                        c.initial_phase.unit.suffix()
                    );
                    if i < 4 {
                        rows.push((
                            key(
                                "period",
                                ["period_1", "period_2", "period_3", "period_4"][i],
                            ),
                            period,
                        ));
                        rows.push((
                            key(
                                "waveform",
                                ["waveform_1", "waveform_2", "waveform_3", "waveform_4"][i],
                            ),
                            c.waveform.label().into(),
                        ));
                        rows.push((
                            key("phase", ["phase_1", "phase_2", "phase_3", "phase_4"][i]),
                            phase,
                        ));
                    }
                }
                if let Some(g) = spec.geometry() {
                    rows.push((
                        "rotation",
                        format!(
                            "{} {}",
                            format_number(g.rotation.value),
                            g.rotation.unit.suffix()
                        ),
                    ));
                    rows.push((
                        "center_px",
                        format!("{}, {}", g.center_offset_px[0], g.center_offset_px[1]),
                    ));
                }
                if let PatternSpec::MultiWave {
                    stripe_width_px,
                    reverse_band_order,
                    ..
                } = spec
                {
                    rows.push((
                        "band_px",
                        stripe_width_px.map_or_else(|| "default".into(), |w| w.to_string()),
                    ));
                    rows.push((
                        "band_order",
                        if *reverse_band_order {
                            "reversed".into()
                        } else {
                            "normal".into()
                        },
                    ));
                }
            }
            None => rows.push((
                "condition",
                record.map_or_else(dash, |r| ui_text(&r.summary)),
            )),
        }
        rows.push((
            "frame_sha256",
            receipt.map_or_else(dash, |r| {
                r.digest.get(..16).unwrap_or(&r.digest).to_string()
            }),
        ));
        rows.push((
            "verification",
            receipt.map_or_else(dash, |r| match r.verification {
                slm_device::Verification::Simulated => "simulated".into(),
                slm_device::Verification::DesktopReadback => "desktop readback".into(),
            }),
        ));
        rows.push((
            "applied_utc",
            receipt.map_or_else(dash, |r| {
                let s = r.at_unix_ms / 1000 % 86_400;
                format!("{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
            }),
        ));
        if let Some(e) = error {
            rows.push(("error", e));
        }
        rows.push(("reset_code", self.config.profile.reset_code.to_string()));
        rows
    }

    fn applied_table(&self, ui: &mut egui::Ui, area: Rect) {
        let inner = area.shrink(2.);
        let first = 104.;
        let header = rect(inner.min.x, inner.min.y, inner.width(), 24.);
        ui.painter().rect_stroke(
            area,
            0.,
            egui::Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_filled(header, 0., HEADER_FILL);
        ui.painter().line_segment(
            [
                Pos2::new(inner.min.x + first, inner.min.y),
                Pos2::new(inner.min.x + first, inner.max.y),
            ],
            egui::Stroke::new(1_f32, BORDER),
        );
        for (text, x) in [
            ("Field", inner.min.x + first * 0.5),
            ("Value", inner.min.x + first + (inner.width() - first) * 0.5),
        ] {
            ui.painter().text(
                Pos2::new(x, header.center().y),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.),
                TEXT,
            );
        }
        let body = Rect::from_min_max(Pos2::new(inner.min.x, header.max.y), inner.max);
        let rows = self.applied_fields();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(body));
                egui::ScrollArea::vertical()
                    .id_salt("applied-table")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        let width = ui.available_width();
                        for (key, value) in rows {
                            let (row, response) =
                                ui.allocate_exact_size(Vec2::new(width, 30.), egui::Sense::hover());
                            let key_cell =
                                rect(row.min.x + 6., row.min.y, first - 10., row.height());
                            let value_cell = rect(
                                row.min.x + first + 6.,
                                row.min.y,
                                row.width() - first - 10.,
                                row.height(),
                            );
                            for (cell, text) in
                                [(key_cell, key.to_string()), (value_cell, value.clone())]
                            {
                                let galley = ui.painter().layout_no_wrap(
                                    text,
                                    egui::FontId::proportional(12.),
                                    TEXT,
                                );
                                ui.painter()
                                    .with_clip_rect(cell.intersect(ui.clip_rect()))
                                    .galley(
                                        Pos2::new(
                                            cell.min.x,
                                            cell.center().y - galley.size().y * 0.5,
                                        ),
                                        galley,
                                        TEXT,
                                    );
                            }
                            response.on_hover_text(value);
                        }
                    });
            },
        );
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
