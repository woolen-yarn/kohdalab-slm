//! Center operation card: the tab row with the External control state,
//! the form dispatch, and the Output + Run side panel.

use super::{
    EXTERNAL_AVAILABLE, EXTERNAL_AVAILABLE_TIP, EXTERNAL_LEASED, EXTERNAL_LEASED_TIP,
    EXTERNAL_UNVERIFIED, EXTERNAL_UNVERIFIED_TIP, EXTERNAL_WAITING, EXTERNAL_WAITING_TIP, H,
};
use crate::app::{App, Tab};
use crate::layout::{Form, Tone, rect};
use crate::theme::{self, AMBER, BORDER, ERROR, MUTED, TEAL, TEXT};
use eframe::egui::{self, Pos2, Rect, RichText, Vec2};
use slm_core::PatternSpec;
use slm_device::backend::BackendKind;
use slm_device::worker::DeviceState;

impl App {
    // ----- center --------------------------------------------------------

    pub(super) fn operation(&mut self, ui: &mut egui::Ui, area: Rect) {
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
        if self.external_control() {
            // Restrained state text as in Camera, with Stop (always) and
            // Take control (while a client holds the lease).
            let leased = self.control.lease.is_some();
            let (text, color, tip) = if leased {
                (EXTERNAL_LEASED, AMBER, EXTERNAL_LEASED_TIP)
            } else if self.control.running().is_some() {
                (EXTERNAL_WAITING, AMBER, EXTERNAL_WAITING_TIP)
            } else if self.output_unverified.is_some()
                || matches!(self.device_state, DeviceState::Unknown { .. })
            {
                (EXTERNAL_UNVERIFIED, ERROR, EXTERNAL_UNVERIFIED_TIP)
            } else {
                (EXTERNAL_AVAILABLE, MUTED, EXTERNAL_AVAILABLE_TIP)
            };
            let can_stop = leased || !matches!(self.device_state, DeviceState::Disconnected);
            let (mut stop, mut take) = (false, false);
            let w = 400_f32.min((area.max.x - x - 8.).max(0.));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect(area.max.x - w - 6., area.min.y, w, 24.))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    fn button(label: &str) -> egui::Button<'static> {
                        egui::Button::new(RichText::new(label).size(12.))
                            .min_size(Vec2::new(0., 22.))
                            .corner_radius(6)
                    }
                    stop = ui
                        .add_enabled(can_stop, button("Stop"))
                        .on_hover_text(
                            "End any external lease, send the reset pattern, then close the output",
                        )
                        .on_disabled_hover_text("Nothing to stop: disconnected and no lease")
                        .clicked();
                    take = ui
                        .add_enabled(leased, button("Take control"))
                        .on_hover_text("End the external lease; the output stays as it is")
                        .on_disabled_hover_text("No client holds the lease")
                        .clicked();
                    ui.add(egui::Label::new(RichText::new(text).size(12.).color(color)).truncate())
                        .on_hover_text(tip);
                },
            );
            if stop {
                self.gui_stop();
            }
            if take {
                self.gui_take_control();
            }
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
                can_save && self.lease_block().is_none(),
                Tone::Neutral,
                Some("Save the current Input Pattern"),
            ) {
                self.save_preview(png);
            }
        }
    }

    pub(super) fn validation_line(&self, f: &mut Form, w: f32) {
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
}
