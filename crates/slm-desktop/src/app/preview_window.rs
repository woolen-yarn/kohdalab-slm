//! Input Pattern preview (current source, sweep validation, render results,
//! saving), the startup window fit and the demo launch capture.

use super::{App, ApplyStatus, Level, SweepBase, Tab, ui_text};
use eframe::egui;
use slm_core::sweep::SweepTarget;
use slm_core::units::format_number;
use std::time::Duration;

impl App {
    pub(super) fn save_preview(&mut self, png: bool) {
        let Some((_, source, caption)) = self.current_preview() else {
            return;
        };
        let (name, extension) = if png {
            ("SLM code heatmap", "png")
        } else {
            ("Santec CSV", "csv")
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("pattern.{extension}"))
            .add_filter(name, &[extension])
            .save_file()
        else {
            return;
        };
        let result = source.save_to(&path, png);
        match result {
            Ok(()) => self.push_log(
                Level::Info,
                format!("Saved {caption} to {}", path.display()),
            ),
            Err(error) => self.push_log(
                Level::Error,
                format!("Save {} failed: {error}", path.display()),
            ),
        }
    }

    /// What the Input Pattern area shows for the current tab: the key, the
    /// source and a caption naming the preview target.
    pub fn current_preview(&self) -> Option<(String, crate::preview::PreviewSource, String)> {
        use crate::preview::PreviewSource;
        let profile = self.config.profile.generation;
        let item_source = |i: usize| {
            let item = self.experiment.experiment.items.get(i)?.clone();
            let caption = format!("List row #{} · {}", i + 1, item.label);
            let asset = self.experiment.asset_path(&item);
            Some((PreviewSource::Item { item, asset }, caption))
        };
        let (source, caption) = match self.tab {
            Tab::Uniform => (
                PreviewSource::Spec {
                    spec: self.uniform_spec(),
                    profile,
                },
                "Uniform form".to_string(),
            ),
            Tab::Single => (
                PreviewSource::Spec {
                    spec: self.single_spec(),
                    profile,
                },
                "Single Wave form".to_string(),
            ),
            Tab::Multi => (
                PreviewSource::Spec {
                    spec: self.multi_spec(),
                    profile,
                },
                "Multi-Wave form".to_string(),
            ),
            Tab::Sweep if self.sweep_preview_row && self.selected.is_some() => {
                item_source(self.selected?)?
            }
            Tab::Sweep => {
                let spec = self.sweep_spec();
                let target = spec.target.label(&spec.base);
                let unit = spec.target.unit(&spec.base).unwrap_or_default();
                match spec.first_pattern().and_then(|(value, first)| {
                    spec.summary().map(|summary| (value, first, summary.count))
                }) {
                    Ok((value, first, count)) => (
                        PreviewSource::Spec {
                            spec: first,
                            profile,
                        },
                        format!(
                            "Sweep first value (1 of {}) · {} = {} {}",
                            count,
                            ui_text(&target),
                            format_number(value),
                            ui_text(&unit)
                        ),
                    ),
                    Err(_) => (
                        PreviewSource::Spec {
                            spec: spec.base,
                            profile,
                        },
                        "Sweep base (sweep values invalid)".to_string(),
                    ),
                }
            }
            Tab::Experiment => item_source(self.selected?)?,
        };
        Some((source.key(), source, caption))
    }

    /// Every sweep value validated against the profile; cached per draft.
    pub fn sweep_validation(&self) -> Result<(), String> {
        let spec = self.sweep_spec();
        let profile = self.config.profile.generation;
        let key = serde_json::to_string(&(&spec, &profile)).unwrap_or_default();
        if let Some((k, result)) = self.sweep_check.borrow().as_ref()
            && *k == key
        {
            return result.clone();
        }
        let result = spec.expand().and_then(|items| {
            items.iter().try_for_each(|(v, s)| {
                s.validate(&profile)
                    .map_err(|e| format!("At {}: {e}", format_number(*v)))
            })
        });
        *self.sweep_check.borrow_mut() = Some((key, result.clone()));
        result
    }

    pub fn preview_ready(&self) -> bool {
        match self.current_preview() {
            Some((key, _, _)) => self.preview.shown.as_deref() == Some(key.as_str()),
            None => true,
        }
    }

    /// Request a render when the input changed; accept only the result for
    /// the latest request. Independent of the device worker.
    pub(super) fn process_previews(&mut self, ctx: &egui::Context) {
        if let Some((key, source, _)) = self.current_preview()
            && self.preview.requested.as_deref() != Some(key.as_str())
        {
            self.preview.requested = Some(key.clone());
            self.previewer.request(key, source);
        }
        for result in self.previewer.try_results() {
            if self.preview.requested.as_deref() != Some(result.key.as_str()) {
                continue;
            }
            match result.result {
                Ok(image) => {
                    let color = egui::ColorImage::from_rgb(
                        [image.width as usize, image.height as usize],
                        &image.rgb,
                    );
                    let options = egui::TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Linear,
                        wrap_mode: egui::TextureWrapMode::ClampToEdge,
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                    };
                    match &mut self.preview.texture {
                        Some(texture) => texture.set(color, options),
                        None => {
                            self.preview.texture =
                                Some(ctx.load_texture("input-pattern", color, options))
                        }
                    }
                    self.preview.info = Some(crate::preview::PreviewImage {
                        rgb: Vec::new(),
                        ..image
                    });
                    self.preview.shown = Some(result.key);
                    self.preview.error = None;
                }
                Err(e) => {
                    self.preview.shown = None;
                    self.preview.error = Some((result.key, e));
                }
            }
        }
    }

    /// Fit the startup window to the monitor it opened on, once. Explicit
    /// capture sizes are kept exactly.
    pub(super) fn fit_window_once(&mut self, ctx: &egui::Context) {
        if self.window_fit.done || self.capture.is_some() {
            return;
        }
        self.window_fit.frames += 1;
        let (monitor, inner) = ctx.input(|i| {
            let v = i.viewport();
            (v.monitor_size, v.inner_rect.map(|r| r.size()))
        });
        match monitor {
            Some(monitor) if monitor.x > 0. && monitor.y > 0. => {
                let requested = inner.unwrap_or(egui::Vec2::new(1566., 900.));
                self.apply_window_fit(ctx, crate::layout::fit_window(monitor, requested));
            }
            // No monitor information (headless, some remote sessions): keep
            // the requested size and the normal minimum.
            _ if self.window_fit.frames > 30 => self.window_fit.done = true,
            _ => ctx.request_repaint(),
        }
    }

    pub fn apply_window_fit(&mut self, ctx: &egui::Context, fit: crate::layout::WindowFit) {
        self.window_fit.done = true;
        self.window_fit.limit = Some(fit.usable);
        self.right_collapsed |= fit.collapse_right;
        self.left_collapsed |= fit.collapse_left;
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(fit.min_inner));
        if fit.maximize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        self.push_log(
            Level::Info,
            if fit.maximize {
                format!(
                    "Window fitted to the screen ({:.0} × {:.0} points usable){}.",
                    fit.usable.x,
                    fit.usable.y,
                    match (fit.collapse_left, fit.collapse_right) {
                        (true, _) => "; side panels folded",
                        (false, true) => "; log panel folded",
                        _ => "",
                    }
                )
            } else {
                "Window fits the screen at 1566 × 900.".into()
            },
        );
    }

    /// Capture-only view selection: final tab and expanded cards.
    pub fn set_capture_view(&mut self, view: &str) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        capture.tab = Tab::ALL.into_iter().find(|t| {
            view.to_lowercase()
                .starts_with(&t.label().to_lowercase().replace(' ', "-"))
        });
        if view.split_whitespace().any(|w| w == "expanded") {
            self.device_expanded = true;
        }
        if view.split_whitespace().any(|w| w == "left-collapsed") {
            self.left_collapsed = true;
        }
        if view.split_whitespace().any(|w| w == "right-collapsed") {
            self.right_collapsed = true;
        }
        if view.split_whitespace().any(|w| w == "settings") {
            self.settings_open = true;
        }
    }

    /// Launch check driven frame by frame (demo only).
    pub(super) fn capture_step(&mut self, ctx: &egui::Context) {
        let Some(mut capture) = self.capture.take() else {
            return;
        };
        capture.frames += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
        match capture.stage {
            0 if !self.displays.is_empty() && capture.frames > 5 => {
                self.connect();
                capture.stage = 1;
            }
            1 if self.connected() => {
                self.tab = Tab::Single;
                self.single.geometry.rotation.value = 30.;
                let spec = self.single_spec();
                self.apply_spec("Single Wave".into(), spec);
                capture.stage = 2;
            }
            2 if matches!(self.status, ApplyStatus::Applied(..)) => {
                let spec = self.single_spec();
                self.add_to_experiment("Single Wave 30°".into(), spec);
                self.sweep.base = SweepBase::Uniform;
                self.sweep.target = SweepTarget::UniformCode;
                self.sweep.start = 0.;
                self.sweep.end = 1023.;
                self.sweep.step = 100.;
                self.sweep.include_end = true;
                self.add_sweep();
                self.apply_experiment_item(0);
                capture.stage = 10;
            }
            10 if self.pending.is_none() => {
                if let Some(index) = self.adjacent_row(true) {
                    self.apply_experiment_item(index);
                }
                capture.stage = 11;
            }
            11 if self.pending.is_none() => {
                self.set_selection(self.showing_index());
                self.sweep_preview_row = false;
                if let Some(tab) = capture.tab {
                    self.tab = tab;
                }
                capture.frames = 0;
                capture.stage = 3;
            }
            3 if capture.frames > 8 && self.preview_ready() => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                capture.stage = 4;
            }
            4 => {
                let image = ctx.input(|i| {
                    i.raw.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = image {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    capture.result = Some(
                        image::RgbaImage::from_raw(
                            image.size[0] as u32,
                            image.size[1] as u32,
                            rgba,
                        )
                        .ok_or_else(|| "Screenshot size mismatch".to_string())
                        .and_then(|img| img.save(&capture.path).map_err(|e| e.to_string())),
                    );
                    match &capture.result {
                        Some(Ok(())) => eprintln!("Captured {}", capture.path.display()),
                        Some(Err(e)) => eprintln!("Capture failed: {e}"),
                        None => {}
                    }
                    capture.stage = 5;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            _ if capture.frames > 600 && capture.stage < 5 => {
                eprintln!("Capture timed out at stage {}", capture.stage);
                capture.stage = 5;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
        self.capture = Some(capture);
    }
}
