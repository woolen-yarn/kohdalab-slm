//! Input Pattern area: the SLM code heatmap preview of the current input.

use crate::app::{App, Tab};
use crate::layout::{Form, rect};
use crate::theme::{self, BORDER, ERROR, MUTED, TEXT};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use slm_core::units::format_number;

impl App {
    /// Lower center: the SLM code heatmap of the current input, as TRKR's
    /// plot area. Image +X is right (column 0 left), +Y is down (row 0 top),
    /// square SLM pixels, color scale 0 … profile max code.
    pub(super) fn input_pattern(&mut self, ui: &mut egui::Ui, area: Rect) {
        let current = self.current_preview();
        let mut use_row = self.sweep_preview_row;
        {
            let mut f = Form::new(ui, area.min);
            f.label(rect(0., 0., 92., 24.), "Input Pattern");
            let caption = current
                .as_ref()
                .map_or_else(|| "No list row selected".to_string(), |(_, _, c)| c.clone());
            let mut caption_w = area.width() - 98.;
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
        let (left, right, top, bottom) = (72., 24., 22., 46.);
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
                "{} × {} px · origin (0, 0) at the image center, pixel ({:.1}, {:.1}) · {} per px\ncodes in use {}–{}, color scale 0–{} (profile max)\nframe sha256 {}…\nPreview only: identical to what Apply would send, but nothing is sent.",
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
                info.digest.get(..16).unwrap_or(&info.digest),
            ));
    }
}
