//! Right rail: the Log above the Field/Value table of the applied condition.

use super::HEADER_FILL;
use crate::app::{App, ApplyStatus, Level, ui_text};
use crate::layout::rect;
use crate::theme::{self, AMBER, BORDER, ERROR, TEXT};
use eframe::egui::{self, Pos2, Rect, RichText, Vec2};
use slm_core::PatternSpec;
use slm_core::units::format_number;
use slm_device::worker::DeviceState;

impl App {
    // ----- right rail ----------------------------------------------------

    pub(super) fn log_view(&mut self, ui: &mut egui::Ui, area: Rect) {
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
        let status_h = if self.demo { 20. } else { 0. };
        if self.demo {
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

    pub(super) fn applied_table(&self, ui: &mut egui::Ui, area: Rect) {
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
