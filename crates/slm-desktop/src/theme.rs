//! KohdaLab family appearance (tokens sampled from IV/TRKR/PLE/Shutter),
//! applied at one seam like TRKR's `precision::configure`.

use eframe::egui::{self, Color32, Vec2};

pub const BACKGROUND: Color32 = Color32::from_rgb(0x12, 0x13, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x1A, 0x1B, 0x20);
pub const INPUT: Color32 = Color32::from_rgb(0x14, 0x15, 0x19);
pub const BUTTON: Color32 = Color32::from_rgb(0x2D, 0x2E, 0x35);
pub const BORDER: Color32 = Color32::from_rgb(0x38, 0x3A, 0x43);
pub const TEXT: Color32 = Color32::from_rgb(0xEB, 0xEC, 0xF1);
pub const MUTED: Color32 = Color32::from_rgb(0xA0, 0xA3, 0xB0);
pub const SELECTION: Color32 = Color32::from_rgb(0x30, 0x5B, 0x9E);
pub const PRIMARY: Color32 = Color32::from_rgb(0x30, 0x6B, 0xD6);
pub const TEAL: Color32 = Color32::from_rgb(0x3F, 0xD7, 0xC2);
pub const AMBER: Color32 = Color32::from_rgb(0xF5, 0xB7, 0x52);
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x7B, 0x82);

pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if cfg!(windows) {
        let windows = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:\\Windows".into());
        if let Ok(bytes) = std::fs::read(windows.join("Fonts").join("segoeui.ttf")) {
            fonts.font_data.insert(
                "windows-ui".into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "windows-ui".into());
        }
    }
    ctx.set_fonts(fonts);
    // The app is always dark, whatever the OS reports (egui otherwise follows
    // the system theme and switches to its unstyled light style). Only this
    // app's preference changes; the Windows theme is untouched. Both style
    // slots get the same dark KohdaLab style so no light widget can appear.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        for (text_style, size) in style.text_styles.iter_mut() {
            *size = match text_style {
                egui::TextStyle::Heading => egui::FontId::proportional(15.),
                egui::TextStyle::Small => egui::FontId::proportional(11.),
                egui::TextStyle::Monospace => egui::FontId::monospace(12.),
                _ => egui::FontId::proportional(12.),
            };
        }
        // TRKR precision::configure values.
        style.spacing.interact_size = Vec2::new(80., 26.);
        style.spacing.button_padding = Vec2::new(6., 3.);
        style.spacing.item_spacing = Vec2::new(6., 6.);
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = 12.;
        style.spacing.scroll.bar_inner_margin = 0.;
        style.spacing.scroll.bar_outer_margin = 0.;
        let v = &mut style.visuals;
        v.panel_fill = BACKGROUND;
        v.window_fill = PANEL;
        v.window_corner_radius = 12.into();
        v.window_stroke = egui::Stroke::new(1_f32, BORDER);
        v.extreme_bg_color = INPUT;
        v.text_edit_bg_color = Some(INPUT);
        v.faint_bg_color = PANEL;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = SELECTION;
        v.selection.stroke = egui::Stroke::new(1_f32, TEXT);
        for widget in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
        ] {
            widget.bg_stroke = egui::Stroke::new(1_f32, BORDER);
            widget.corner_radius = 6.into();
            widget.fg_stroke = egui::Stroke::new(1_f32, TEXT);
        }
        v.widgets.inactive.bg_fill = BUTTON;
        v.widgets.inactive.weak_bg_fill = BUTTON;
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x38, 0x39, 0x42);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1_f32, BORDER);
        v.widgets.noninteractive.corner_radius = 6.into();
    });
}
