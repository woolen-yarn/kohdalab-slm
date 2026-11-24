//! Left rail device UI: the Config card, the SLM device card and the device
//! settings window with its wavelength rows.

use super::{GREEN, H, ROW, expand_home};
use crate::app::{App, Level};
use crate::layout::{Form, Tone, rect};
use crate::theme::{AMBER, ERROR, MUTED, TEAL, TEXT};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use slm_device::backend::BackendKind;
use slm_device::demo::demo_displays;
use slm_device::worker::DeviceState;
use slm_device::{AppConfig, InputMode, ModeConfirmation};

impl App {
    pub(super) fn config_card(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) -> f32 {
        let inner = w - 22.;
        let half = ((inner - 8.) / 2.).round();
        let busy = self.device_busy();
        let can_open_file_dialog = !self.demo && self.lease_block().is_none();
        let (browse, load, save);
        {
            let mut f = Form::new(ui, origin);
            f.group(rect(0., 0., w, 96.), "Config");
            f.path(
                "config-path",
                rect(11., 27., inner - 78., H),
                &mut self.config_path_text,
            );
            browse = f.button(rect(w - 81., 27., 70., H), "Browse", can_open_file_dialog);
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
                can_open_file_dialog,
                Tone::Neutral,
                Some(if self.demo {
                    "Demo config is temporary"
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
            match AppConfig::load(&path) {
                Ok((config, note)) => {
                    if let Some(note) = note {
                        self.push_log(Level::Warn, note);
                    }
                    self.config = config;
                    self.config_path = path;
                    self.config_dirty = false;
                    self.displays.clear();
                    self.refresh_displays();
                    self.push_log(
                        Level::Info,
                        format!("Config loaded: {}", self.config_path.display()),
                    );
                }
                Err(e) => self.push_log(Level::Error, format!("Config not loaded: {e}")),
            }
        }
        if save {
            self.config_path = expand_home(self.config_path_text.trim());
            self.save_config();
        }
        96.
    }

    pub(super) fn state_caption(&self) -> (String, Color32) {
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
    pub(super) fn slm_card(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) -> f32 {
        let busy = self.device_busy();
        let connected = self.connected();
        let (caption, caption_color) = self.state_caption();
        let leased = self.lease_block();
        let block = leased.map(String::from).or_else(|| self.connect_block());
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
                self.config.output.display = None;
                self.displays.clear();
                self.refresh_displays();
                self.save_config();
            }
            y += ROW;
            f.label(rect(lx, y, col_width, H), "Mode");
            match self.config.output.confirmed_mode.clone() {
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
                    let ok = self.mode_ack
                        && self.mode_choice.supported()
                        && !self.mode_note.trim().is_empty();
                    if f.button_with(
                        rect(action_x, y, col_width, H),
                        "Confirm",
                        ok,
                        Tone::Neutral,
                        Some("Record the verified input mode (60 Hz, Table 6)"),
                    ) {
                        self.config.output.confirmed_mode = Some(ModeConfirmation {
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
            ) && self.pending.is_none()
                && leased.is_none(),
            Tone::Neutral,
            Some(leased.unwrap_or("Send the reset pattern, then close the output")),
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
            connected && self.pending.is_none() && leased.is_none(),
            Tone::Neutral,
            Some(&leased.map(String::from).unwrap_or_else(|| {
                format!(
                    "Show uniform raw code {} now (not verified dark)",
                    self.config.profile.reset_code
                )
            })),
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

    pub(super) fn device_settings_window(&mut self, ctx: &egui::Context, origin: Pos2) {
        let mut open = true;
        let width = 352.;
        egui::Window::new("Device Settings")
            .id(egui::Id::new("device-settings-window"))
            .default_pos(origin)
            .default_width(width)
            .resizable(false)
            .collapsible(false)
            .vscroll(true)
            .open(&mut open)
            .show(ctx, |ui| {
                let start = ui.available_rect_before_wrap().min;
                let mut form = Form::new(ui, start);
                let used = self.settings_rows(&mut form, width, 4.);
                ui.set_min_size(Vec2::new(width, used));
            });
        self.settings_open = open;
    }

    fn settings_rows(&mut self, f: &mut Form, w: f32, mut y: f32) -> f32 {
        let locked = self.device_busy();
        let before = self.config.clone();
        f.note(
            rect(11., y, w - 22., H),
            if locked {
                "Disconnect to edit · saved by Config Save"
            } else {
                "Saved by Config Save"
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
        row(f, "phase0", "code", &mut y, &mut |f, r| {
            f.number("phase0", r, &mut p.generation.calibration.phase0, true);
        });
        row(f, "Code period", "codes / 2π", &mut y, &mut |f, r| {
            f.integer(
                "code-period",
                r,
                &mut p.generation.calibration.code_period,
                1.,
                1024.,
            );
        });
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
        row(f, "Center X / Y", "px", &mut y, &mut |f, r| {
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
        f.text("wavelength", rect(114., y, 86., H), &mut self.wavelength_nm);
        f.muted(rect(206., y, 22., H), "nm");
        if !simulated {
            f.label(rect(232., y, 46., H), "USB #");
            f.text(
                "wavelength-device",
                rect(279., y, w - 290., H),
                &mut self.wavelength_usb_device,
            );
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
                "Write to the selected USB device and verify by readback"
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
}
