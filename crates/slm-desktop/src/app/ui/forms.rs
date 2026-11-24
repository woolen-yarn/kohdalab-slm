//! Operation tab forms: Uniform, Single Wave, Multi-Wave, Sweep and
//! Experiment, with their shared period / phase / rotation / center rows.

use super::H;
use crate::app::{App, Discard, SweepBase, ui_text};
use crate::layout::{Form, Tone, rect};
use crate::theme::{AMBER, ERROR, MUTED, TEXT};
use eframe::egui::{self, Pos2, Rect, Vec2};
use slm_core::experiment::ExperimentDoc;
use slm_core::heatmap::HeatmapKind;
use slm_core::sweep::SweepTarget;
use slm_core::units::format_number;
use slm_core::{
    AngleUnit, Geometry, PeriodUnit, PeriodValue, PhaseUnit, PhaseValue, WaveComponent, Waveform,
};
use std::path::PathBuf;

impl App {
    pub(super) fn uniform_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
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

    pub(super) fn single_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
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

    pub(super) fn multi_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
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

    pub(super) fn sweep_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
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
        rfd::FileDialog::new()
            .add_filter("KohdaLab SLM experiment", &["json"])
            .set_file_name(format!("{}.json", self.experiment.experiment.name))
            .save_file()
    }

    pub(super) fn experiment_form(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32) {
        let path = self
            .experiment
            .path
            .as_ref()
            .map_or_else(|| "Not saved".to_string(), |p| p.display().to_string());
        let can_open_file_dialog = self.lease_block().is_none();
        let mut f = Form::new(ui, origin);
        f.label(rect(9., 9., 66., H), "Name");
        f.text(
            "experiment-name",
            rect(81., 9., 200., H),
            &mut self.experiment.experiment.name,
        );
        if self.experiment.dirty {
            f.value(rect(289., 9., 80., H), "unsaved", AMBER);
        }
        f.label(rect(9., 41., 66., H), "File");
        f.path_summary(rect(81., 41., w - 90., H), &path);
        let bw = 96.;
        let buttons = ["New List", "Open List…", "Save List", "Save List As…"];
        let mut clicked = None;
        for (i, name) in buttons.iter().enumerate() {
            let enabled = i == 0 || can_open_file_dialog;
            if f.button(rect(81. + i as f32 * (bw + 6.), 73., bw, H), name, enabled) {
                clicked = Some(*name);
            }
        }
        let import = f.button(rect(81., 105., bw, H), "Import CSV…", can_open_file_dialog);
        let export = f.button(
            rect(81. + bw + 6., 105., bw, H),
            "Export…",
            can_open_file_dialog && !self.experiment.experiment.items.is_empty(),
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
        let mut discard = None;
        if let Some(action) = self.confirm_discard {
            f.value(
                rect(9., 169., 240., H),
                "Unsaved list changes will be lost.",
                AMBER,
            );
            let can_discard = matches!(action, Discard::New) || can_open_file_dialog;
            if f.button(
                rect(255., 169., 140., H),
                "Discard and Continue",
                can_discard,
            ) {
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
                if let Some(p) = rfd::FileDialog::new()
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
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("KohdaLab SLM experiment", &["json"])
                        .pick_file()
                    {
                        self.open_experiment(&p);
                    }
                }
            }
        }
        if import
            && let Some(files) = rfd::FileDialog::new()
                .add_filter("Santec CSV", &["csv"])
                .pick_files()
        {
            for file in files {
                self.import_csv(&file);
            }
        }
        if export && let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.export_experiment(&dir);
        }
    }
}
