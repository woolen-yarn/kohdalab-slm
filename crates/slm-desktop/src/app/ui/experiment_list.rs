//! Shared experiment list card below the operation card: row captions, the
//! preview status lines, and selecting, applying, reordering and removing rows.

use super::HEADER_FILL;
use crate::app::{App, ApplyStatus, ui_text};
use crate::layout::{Form, Tone, rect};
use crate::theme::{self, AMBER, BORDER, ERROR, MUTED, TEAL, TEXT};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2};
use slm_core::experiment::ExperimentItem;
use slm_core::units::format_number;
use slm_device::worker::DeviceState;

impl App {
    // ----- shared list ---------------------------------------------------

    fn pattern_label(item: &ExperimentItem) -> String {
        item.sweep.as_ref().map_or_else(
            || item.label.clone(),
            |sweep| {
                let unit = ui_text(&sweep.unit);
                format!("{} {unit}", format_number(sweep.value))
                    .trim_end()
                    .into()
            },
        )
    }

    pub(super) fn row_caption(&self, index: usize) -> String {
        let item = &self.experiment.experiment.items[index];
        format!("Row #{} · {}", index + 1, Self::pattern_label(item))
    }

    pub(super) fn preview_status(&self) -> [(String, Color32); 3] {
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
    pub(super) fn list_card(&mut self, ui: &mut egui::Ui, origin: Pos2, w: f32, height: f32) {
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
                Some(block.unwrap_or("Apply the focused list row")),
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
            let half = (w - 28.) / 2.;
            remove = f.button_with(
                rect(11., footer, half, 24.),
                "Remove Selected",
                selected_count > 0,
                Tone::Neutral,
                Some("Remove selected rows from the list"),
            );
            if f.button(rect(17. + half, footer, half, 24.), "Remove All…", n > 0) {
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
}
