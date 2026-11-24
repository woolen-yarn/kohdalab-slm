//! Experiment workflow: list selection and editing, the form specs, adding
//! patterns and sweeps, and experiment files (import, save, open, export).

use super::{App, Level, SweepBase};
use eframe::egui;
use slm_core::PatternSpec;
use slm_core::experiment::{ExperimentDoc, ExperimentItem, SweepOrigin, new_id};
use slm_core::heatmap::HeatmapKind;
use slm_core::sweep::SweepSpec;
use slm_core::units::format_number;
use slm_device::worker::DeviceState;
use std::collections::BTreeSet;
use std::path::Path;

impl App {
    /// Index of the experiment item the worker reports as shown.
    pub fn showing_index(&self) -> Option<usize> {
        let DeviceState::Connected { showing, .. } = &self.device_state else {
            return None;
        };
        let id = showing.item_id.as_ref()?;
        self.experiment
            .experiment
            .items
            .iter()
            .position(|i| &i.id == id)
    }

    pub(super) fn adjacent_row(&self, forward: bool) -> Option<usize> {
        let len = self.experiment.experiment.items.len();
        let index = match (self.selected.or_else(|| self.showing_index()), forward) {
            (Some(i), true) => i.checked_add(1)?,
            (Some(i), false) => i.checked_sub(1)?,
            (None, true) => 0,
            (None, false) => len.checked_sub(1)?,
        };
        (index < len).then_some(index)
    }

    pub(super) fn set_selection(&mut self, index: Option<usize>) {
        self.queued_row = None;
        self.selected = index;
        self.selection_anchor = index;
        self.selected_rows.clear();
        self.selected_rows.extend(index);
    }

    pub(super) fn select_row(&mut self, index: usize, modifiers: egui::Modifiers) {
        self.queued_row = None;
        if modifiers.shift {
            let anchor = *self.selection_anchor.get_or_insert(index);
            if !modifiers.command {
                self.selected_rows.clear();
            }
            self.selected_rows
                .extend(anchor.min(index)..=anchor.max(index));
            self.selected = Some(index);
        } else if modifiers.command {
            self.selection_anchor = Some(index);
            self.selected = if self.selected_rows.remove(&index) {
                self.selected_rows.first().copied()
            } else {
                self.selected_rows.insert(index);
                Some(index)
            };
        } else {
            self.set_selection(Some(index));
            // While external control holds the device a click only selects;
            // nothing is queued to fire after the lease ends.
            if self.auto_apply_rows && self.connected() && self.lease_block().is_none() {
                if self.pending.is_some() {
                    self.queued_row = Some(index);
                } else {
                    self.apply_experiment_item(index);
                }
            }
        }
        self.sweep_preview_row = self.selected.is_some();
    }

    pub(super) fn move_rows(&mut self, dragged_id: &str, gap: usize) {
        self.queued_row = None;
        let items = &mut self.experiment.experiment.items;
        let Some(source) = items.iter().position(|item| item.id == dragged_id) else {
            return;
        };
        let moving = if self.selected_rows.contains(&source) {
            self.selected_rows.clone()
        } else {
            BTreeSet::from([source])
        };
        let start = gap - moving.range(..gap).count();
        if moving.iter().copied().eq(start..start + moving.len()) {
            return;
        }
        let focus = start + moving.range(..source).count();
        let mut block = Vec::with_capacity(moving.len());
        let mut rest = Vec::with_capacity(items.len() - moving.len());
        for (index, item) in std::mem::take(items).into_iter().enumerate() {
            if moving.contains(&index) {
                block.push(item);
            } else {
                rest.push(item);
            }
        }
        rest.splice(start..start, block);
        *items = rest;
        self.selected = Some(focus);
        self.selection_anchor = Some(focus);
        self.selected_rows = (start..start + moving.len()).collect();
        self.reveal_row = Some(focus);
        self.sweep_preview_row = true;
        self.experiment.dirty = true;
    }

    pub(super) fn remove_rows(&mut self, all: bool) {
        let items = &mut self.experiment.experiment.items;
        let count = items.len();
        let first = self.selected_rows.first().copied();
        let mut index = 0;
        items.retain(|item| {
            let remove = all || self.selected_rows.contains(&index);
            index += 1;
            if remove {
                self.experiment.pending_assets.remove(&item.id);
            }
            !remove
        });
        let remaining = items.len();
        if remaining != count {
            self.experiment.dirty = true;
            let next = remaining
                .checked_sub(1)
                .zip(first)
                .map(|(last, i)| i.min(last));
            self.set_selection(next);
            self.reveal_row = next;
            self.push_log(
                Level::Info,
                format!("Removed {} list items.", count - remaining),
            );
        }
    }

    pub fn add_to_experiment(&mut self, label: String, spec: PatternSpec) {
        let profile = self.config.profile.generation;
        if let Err(e) = spec.validate(&profile) {
            self.push_log(Level::Warn, format!("Not added: {e}"));
            return;
        }
        self.experiment
            .push(ExperimentItem::generated(label, spec, profile));
        let index = self.experiment.experiment.items.len() - 1;
        self.set_selection(Some(index));
        self.reveal_row = Some(index);
        self.push_log(Level::Info, "Added 1 item to the experiment.");
    }

    pub fn uniform_spec(&self) -> PatternSpec {
        PatternSpec::Uniform {
            code: self.uniform_code,
        }
    }
    pub fn single_spec(&self) -> PatternSpec {
        PatternSpec::SingleWave {
            wave: self.single.wave,
            geometry: self.single.geometry,
        }
    }
    pub fn multi_spec(&self) -> PatternSpec {
        PatternSpec::MultiWave {
            components: self.multi.components.clone(),
            geometry: self.multi.geometry,
            stripe_width_px: self.multi.band_override.then_some(self.multi.band_width),
            reverse_band_order: self.multi.reverse_band_order,
        }
    }
    pub fn sweep_spec(&self) -> SweepSpec {
        SweepSpec {
            base: match self.sweep.base {
                SweepBase::Uniform => self.uniform_spec(),
                SweepBase::Single => self.single_spec(),
                SweepBase::Multi => self.multi_spec(),
            },
            target: self.sweep.target,
            start: self.sweep.start,
            end: self.sweep.end,
            step: self.sweep.step,
            include_end: self.sweep.include_end,
        }
    }

    pub fn add_sweep(&mut self) {
        let spec = self.sweep_spec();
        let profile = self.config.profile.generation;
        let expanded = spec.expand().and_then(|items| {
            for (value, s) in &items {
                s.validate(&profile)
                    .map_err(|e| format!("Value {}: {e}", format_number(*value)))?;
            }
            Ok(items)
        });
        let items = match expanded {
            Ok(items) => items,
            Err(e) => {
                self.push_log(Level::Warn, format!("Sweep not added: {e}"));
                return;
            }
        };
        let sweep_id = new_id();
        let count = items.len();
        let first_added = self.experiment.experiment.items.len();
        let target_label = spec.target.label(&spec.base);
        let unit = spec.target.unit(&spec.base).unwrap_or_default();
        for (index, (value, s)) in items.into_iter().enumerate() {
            let mut item = ExperimentItem::generated(
                format!(
                    "{target_label} = {} ({}/{count})",
                    format_number(value),
                    index + 1
                ),
                s,
                profile,
            );
            item.sweep = Some(SweepOrigin {
                sweep_id: sweep_id.clone(),
                index,
                count,
                target: spec.target,
                target_label: target_label.clone(),
                value,
                unit: unit.clone(),
            });
            self.experiment.push(item);
        }
        self.reveal_row = Some(first_added);
        self.push_log(
            Level::Info,
            format!("Added sweep of {count} items ({target_label})."),
        );
    }

    pub fn import_csv(&mut self, path: &Path) {
        match self
            .experiment
            .import_csv(path, self.config.profile.generation)
        {
            Ok(_) => {
                let index = self.experiment.experiment.items.len() - 1;
                self.set_selection(Some(index));
                self.reveal_row = Some(index);
                self.push_log(
                    Level::Info,
                    format!(
                        "Imported {} (copied into the experiment on save).",
                        path.display()
                    ),
                );
            }
            Err(e) => self.push_log(Level::Error, format!("Import failed: {e}")),
        }
    }

    pub fn save_experiment(&mut self, path: &Path) {
        match self.experiment.save(path) {
            Ok(()) => self.push_log(Level::Info, format!("Experiment saved: {}", path.display())),
            Err(e) => self.push_log(Level::Error, format!("Experiment not saved: {e}")),
        }
    }

    /// Loading never changes the output display and never sends a pattern.
    pub fn open_experiment(&mut self, path: &Path) {
        match ExperimentDoc::load(path) {
            Ok(doc) => {
                self.push_log(
                    Level::Info,
                    format!(
                        "Opened {} ({} items). Nothing was sent.",
                        path.display(),
                        doc.experiment.items.len()
                    ),
                );
                self.experiment = doc;
                self.set_selection(None);
                self.reveal_row = None;
            }
            Err(e) => self.push_log(Level::Error, format!("Open failed: {e}")),
        }
    }

    pub fn export_experiment(&mut self, dir: &Path) {
        let kinds: Vec<HeatmapKind> = HeatmapKind::ALL
            .into_iter()
            .filter(|k| self.heatmaps.contains(k))
            .collect();
        match self.experiment.export(dir, &kinds) {
            Ok(files) => self.push_log(
                Level::Info,
                format!("Exported {} files to {}", files.len(), dir.display()),
            ),
            Err(e) => self.push_log(Level::Error, format!("Export failed: {e}")),
        }
    }
}
