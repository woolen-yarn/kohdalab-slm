//! Headless GUI checks on the real `App` (demo backend, temporary config).
//! No window, display or device is opened.

use crate::app::{App, AppOptions, ApplyStatus, Tab};
use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable};
use slm_device::worker::DeviceState;
use std::time::{Duration, Instant};

/// TRKR's standard window, also this app's default.
const SIZE: egui::Vec2 = egui::Vec2::new(1566., 900.);

fn harness(demo: bool) -> Harness<'static, App> {
    let dir = tempfile::tempdir().expect("temp config dir");
    Harness::builder()
        .with_size(SIZE)
        .with_max_steps(4)
        .build_eframe(move |cc| {
            App::new(
                &cc.egui_ctx,
                AppOptions {
                    config_path: dir.path().join("config.json"),
                    demo,
                    tempdir: Some(dir),
                    capture: None,
                },
            )
        })
}

fn until(h: &mut Harness<'static, App>, what: &str, mut ok: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        h.run_steps(1);
        if ok(h.state()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn shows(h: &Harness<'static, App>) -> usize {
    h.state().demo_control.lock().shows
}

fn last_code(h: &Harness<'static, App>) -> Option<u16> {
    h.state()
        .demo_control
        .lock()
        .last_frame
        .as_ref()
        .and_then(|f| f.uniform_code())
}

#[test]
fn demo_connect_apply_and_step_through_sweep() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Auto-apply on click").click();
    h.run_steps(2);
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    assert_eq!(shows(&h), 1, "connect shows exactly the reset frame");
    h.get_by_label("Connected · 60 Hz · simulated");

    // The shared list stays visible and changing tabs never sends.
    for tab in [
        "Single Wave",
        "Multi-Wave",
        "Sweep",
        "Experiment",
        "Uniform",
    ] {
        h.get_by_label(tab).click();
        h.run_steps(2);
        h.get_by_label("Apply Row");
        h.get_by_label("Previous");
        h.get_by_label("Next");
        h.get_by_label("Remove Selected");
        assert!(h.query_by_label("Apply Next").is_none());
    }
    assert_eq!(shows(&h), 1);

    h.state_mut().uniform_code = 512;
    h.run_steps(1);
    h.get_by_label("Apply").click();
    until(&mut h, "uniform applied", |a| {
        matches!(a.status, ApplyStatus::Applied(..))
    });
    assert_eq!(last_code(&h), Some(512));

    // Sweep 0 → 20 / 10 → 3 items. Next starts at the first row.
    h.get_by_label("Sweep").click();
    h.run_steps(1);
    {
        let app = h.state_mut();
        app.sweep.start = 0.;
        app.sweep.end = 20.;
        app.sweep.step = 10.;
    }
    h.run_steps(2);
    h.get_by_label_contains("Last 20 code · 3 points");
    h.get_by_label("Add Sweep").click();
    h.run_steps(1);
    assert_eq!(h.state().experiment.experiment.items.len(), 3);
    assert_eq!(h.state().selected, Some(0));
    assert_eq!(last_code(&h), Some(512), "adding sweep does not apply");
    h.get_by_label("Apply Row").click();
    until(&mut h, "first sweep row", |a| {
        a.pending.is_none() && a.showing_index() == Some(0)
    });
    assert_eq!(last_code(&h), Some(0));
    for expected in [10u16, 20] {
        h.get_by_label("Next").click();
        until(&mut h, "next applied", |a| {
            a.pending.is_none() && matches!(a.status, ApplyStatus::Applied(..))
        });
        assert_eq!(last_code(&h), Some(expected));
    }
    let before = shows(&h);
    h.get_by_label("Next").click();
    h.run_steps(2);
    assert_eq!(shows(&h), before, "end of list sends nothing");
    assert_eq!(h.state().tab, Tab::Sweep);
    h.get_by_label("Previous").click();
    until(&mut h, "previous applied", |a| {
        a.pending.is_none() && a.showing_index() == Some(1)
    });
    assert_eq!(last_code(&h), Some(10));

    // Experiment tab: selecting a specific row sends only on Apply Row.
    h.get_by_label("Experiment").click();
    h.run_steps(2);
    h.get_by_label("#1 Contrast level [code] = 0 code").click();
    h.run_steps(2);
    assert_eq!(last_code(&h), Some(10));
    h.get_by_label("Selected · Row #1 · Contrast level [code] = 0 code");
    h.get_by_label("Applied · Row #2 · Contrast level [code] = 10 code");
    h.get_by_label("Save target · Row #1 · Contrast level [code] = 0 code");
    h.get_by_label("Apply Row").click();
    until(&mut h, "selected applied", |a| {
        a.pending.is_none() && a.showing_index() == Some(0)
    });
    assert_eq!(last_code(&h), Some(0));
    let before = shows(&h);
    h.get_by_label("Previous").click();
    h.run_steps(2);
    assert_eq!(shows(&h), before, "start of list sends nothing");

    // A simulated renumbering hides output and reports it as unknown.
    let mut displays = slm_device::demo::demo_displays();
    displays[1].gdi_name = r"\\.\DISPLAY3".into();
    h.state()
        .demo_control
        .simulate_display_change(displays, "Windows renumbered the displays.");
    until(&mut h, "unknown state", |a| {
        matches!(a.device_state, DeviceState::Unknown { .. })
    });
    assert!(h.query_all_by_label("Output unknown").next().is_some());
    h.get_by_label("Disconnect").click();
    until(&mut h, "disconnected", |a| {
        matches!(a.device_state, DeviceState::Disconnected)
    });
}

#[test]
fn unconfirmed_mode_keeps_connect_disabled() {
    let mut h = harness(false);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    let slm = h.state().displays[1].selection();
    h.state_mut().config.output.display = Some(slm);
    h.run_steps(2);
    h.get_by_label("Connect").click();
    h.run_steps(3);
    assert!(matches!(h.state().device_state, DeviceState::Disconnected));
    assert_eq!(shows(&h), 0);
    h.get_by_label_contains("SLM input mode is not confirmed");
    h.get_by_label("Disconnected");
}

#[test]
fn list_supports_range_selection_drag_and_confirmed_removal() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Auto-apply on click").click();
    h.run_steps(2);
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    for (code, label) in ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]
        .into_iter()
        .enumerate()
    {
        h.state_mut().add_to_experiment(
            label.into(),
            slm_core::PatternSpec::Uniform { code: code as u16 },
        );
    }
    h.run_steps(2);
    let labels = |app: &App| {
        app.experiment
            .experiment
            .items
            .iter()
            .map(|item| item.label.clone())
            .collect::<Vec<_>>()
    };

    h.get_by_label("#2 Bravo").click();
    h.run_steps(2);
    h.get_by_label("Apply Row").click();
    until(&mut h, "Bravo applied", |a| a.showing_index() == Some(1));
    let sent_before_editing = shows(&h);
    h.get_by_label("#4 Delta")
        .click_modifiers(egui::Modifiers::SHIFT);
    h.run_steps(2);
    h.get_by_label("5 items · 3 selected");
    h.get_by_label("#3 Charlie")
        .click_modifiers(egui::Modifiers::COMMAND);
    h.run_steps(2);
    h.get_by_label("5 items · 2 selected");

    let from = h.get_by_label("#2 Bravo").rect().center();
    let to = h.get_by_label("#5 Echo").rect().center() + egui::vec2(0., 5.);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(from));
    h.event(button(from, true));
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(2);
    h.event(button(to, false));
    h.run_steps(2);
    assert_eq!(
        labels(h.state()),
        ["Alpha", "Charlie", "Echo", "Bravo", "Delta"]
    );
    assert_eq!(h.state().showing_index(), Some(3));
    h.get_by_label("5 items · 2 selected");

    h.get_by_label("Remove Selected").click();
    h.run_steps(2);
    assert_eq!(labels(h.state()), ["Alpha", "Charlie", "Echo"]);

    h.get_by_label("Remove All…").click();
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(labels(h.state()), ["Alpha", "Charlie", "Echo"]);
    h.get_by_label("Remove All…").click();
    h.run_steps(2);
    h.get_by_label("Remove All Patterns").click();
    h.run_steps(2);
    assert!(labels(h.state()).is_empty());
    assert_eq!(last_code(&h), Some(1));
    assert_eq!(shows(&h), sent_before_editing);
}

#[test]
fn auto_apply_sends_plain_row_clicks_while_connected() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    for (code, label) in [(101, "One"), (202, "Two"), (303, "Three")] {
        h.state_mut()
            .add_to_experiment(label.into(), slm_core::PatternSpec::Uniform { code });
    }
    h.run_steps(2);
    h.get_by_label("#2 Two").click();
    h.run_steps(2);
    assert_eq!(shows(&h), 0, "a disconnected selection is not queued");
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    assert_eq!(last_code(&h), Some(0));
    h.get_by_label("#1 One").click();
    until(&mut h, "row applied by click", |a| {
        a.pending.is_none() && a.showing_index() == Some(0)
    });
    assert_eq!(last_code(&h), Some(101));

    // Hold the simulated backend so several clicks arrive during one send.
    let before_queue = shows(&h);
    let control = h.state().demo_control.clone();
    let hold = control.lock();
    for row in ["#2 Two", "#1 One", "#3 Three"] {
        h.get_by_label(row).click();
        h.run_steps(2);
    }
    drop(hold);
    until(&mut h, "most recent click applied", |a| {
        a.pending.is_none() && a.showing_index() == Some(2)
    });
    assert_eq!(last_code(&h), Some(303));
    assert_eq!(shows(&h), before_queue + 2);

    h.key_press(egui::Key::ArrowUp);
    until(&mut h, "previous row applied by key", |a| {
        a.pending.is_none() && a.showing_index() == Some(1)
    });
    assert_eq!(last_code(&h), Some(202));
    let before = shows(&h);
    h.get_by_label("#2 Two")
        .click_modifiers(egui::Modifiers::SHIFT);
    h.run_steps(3);
    assert_eq!(shows(&h), before, "range selection does not send");
    h.get_by_label("Auto-apply on click").click();
    h.run_steps(2);
    h.get_by_label("#2 Two").click();
    h.run_steps(3);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(h.state().selected, Some(2));
    assert_eq!(
        shows(&h),
        before,
        "turning auto-apply off restores manual selection"
    );
}

#[test]
fn wavelength_demo_reads_and_writes_calibration() {
    let mut h = harness(true);
    h.get_by_label("Settings").click();
    h.run_steps(3);
    h.get_by_label("Read Wavelength").click();
    until(&mut h, "simulated wavelength read", |a| {
        a.config.profile.device_wavelength_nm == Some(795.)
    });
    assert!(
        h.query_all_by_label("Simulated readback: 795 nm")
            .next()
            .is_some()
    );
    let field = h.get_all_by_value("795").next().unwrap();
    field.click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_all_by_value("795").next().unwrap().type_text("810");
    h.run_steps(2);
    h.get_by_label("Set Wavelength").click();
    until(&mut h, "simulated wavelength written", |a| {
        a.config.profile.device_wavelength_nm == Some(810.)
    });
    assert!(
        h.query_all_by_label("Simulated readback: 810 nm")
            .next()
            .is_some()
    );
    assert_eq!(shows(&h), 0);
}

#[test]
fn loading_a_config_replaces_wavelength_input_and_clears_simulated_readback() {
    let mut h = harness(false);
    let mut config = h.state().config.clone();
    config.output.backend = slm_device::backend::BackendKind::Demo;
    config.profile.device_wavelength_nm = Some(830.);
    let path = h.state().config_path.clone();
    config.save(&path).unwrap();
    h.get_by_label("Load").click();
    h.get_by_label("Settings").click();
    h.run_steps(3);
    assert!(h.get_all_by_value("830").next().is_some());
    h.get_by_label("Read Wavelength").click();
    until(&mut h, "simulated wavelength read", |a| {
        a.log
            .iter()
            .any(|line| line.text == "Simulated readback: 795 nm")
    });
    assert_eq!(h.state().config.profile.device_wavelength_nm, Some(830.));
    assert!(h.get_all_by_value("830").next().is_some());
    h.get_by_label("Load").click();
    h.run_steps(3);
    assert!(h.get_all_by_value("830").next().is_some());
    h.get_by_label("Simulated USB calibration");
}

#[test]
fn side_columns_collapse_with_edge_toggles() {
    let mut h = harness(true);
    h.run_steps(2);
    // Expanded: left edge shows "<", right edge shows ">".
    h.get_by_label("<").click();
    h.run_steps(2);
    assert!(h.state().left_collapsed);
    assert!(h.query_by_label("Connect").is_none());
    h.state_mut().left_collapsed = false;
    h.run_steps(2);
    h.get_by_label(">").click();
    h.run_steps(2);
    assert!(h.state().right_collapsed);
}

fn wait_preview(h: &mut Harness<'static, App>) {
    until(h, "preview of the current input", |a| a.preview_ready());
}

#[test]
fn input_pattern_preview_matches_input_and_never_sends() {
    use slm_core::{AngleValue, PatternSpec};
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    let sent_before = shows(&h);
    let profile = h.state().config.profile.generation;

    // Uniform: preview digest equals the frame Apply would send.
    h.state_mut().uniform_code = 300;
    h.run_steps(1);
    wait_preview(&mut h);
    let expected = PatternSpec::Uniform { code: 300 }
        .render(&profile)
        .unwrap()
        .code()
        .digest();
    assert_eq!(h.state().preview.info.as_ref().unwrap().digest, expected);

    // An edit invalidates the shown image until the new render arrives.
    h.state_mut().uniform_code = 301;
    assert!(
        !h.state().preview_ready(),
        "old image must not count as current"
    );
    wait_preview(&mut h);
    assert_eq!(h.state().preview.info.as_ref().unwrap().max_code_used, 301);

    // Single Wave: colorful preview from the same render path.
    h.get_by_label("Single Wave").click();
    h.state_mut().single.geometry.rotation = AngleValue::degrees(30.);
    h.run_steps(1);
    assert!(
        !h.state().preview_ready(),
        "tab switch must not reuse the Uniform image"
    );
    wait_preview(&mut h);
    let spec = h.state().single_spec();
    let info = h.state().preview.info.clone().unwrap();
    assert_eq!(info.digest, spec.render(&profile).unwrap().code().digest());
    assert!(info.max_code_used > info.min_code);

    // Sweep previews the first value, or an explicitly chosen list row.
    h.get_by_label("Sweep").click();
    {
        let app = h.state_mut();
        app.sweep.start = 40.;
        app.sweep.end = 0.;
        app.sweep.step = 10.;
    }
    h.run_steps(2);
    wait_preview(&mut h);
    h.get_by_label_contains("Sweep first value (1 of 5)");
    h.get_by_label("Save target · Sweep first value");
    assert_eq!(h.state().preview.info.as_ref().unwrap().max_code_used, 40);
    h.get_by_label("Add Sweep").click();
    h.run_steps(1);
    h.state_mut().selected = Some(2);
    h.get_by_label("List row").click();
    h.run_steps(2);
    wait_preview(&mut h);
    h.get_by_label_contains("List row #3");
    h.get_by_label("Save target · Row #3 · Contrast level [code] = 20 code");
    assert_eq!(h.state().preview.info.as_ref().unwrap().max_code_used, 20);

    // Experiment: the selected row.
    h.get_by_label("Experiment").click();
    h.state_mut().selected = Some(0);
    h.run_steps(2);
    wait_preview(&mut h);
    assert_eq!(h.state().preview.info.as_ref().unwrap().max_code_used, 40);

    assert_eq!(shows(&h), sent_before, "previews and selections never send");
}

#[test]
fn settings_window_keeps_the_patterns_list_visible() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    // TRKR device-card row: Connect | Disconnect | Settings, plus the
    // Reset Pattern operation row and Display Refresh.
    for label in [
        "Connect",
        "Disconnect",
        "Settings",
        "Reset Pattern",
        "Refresh",
    ] {
        h.get_by_label(label);
    }
    assert!(
        h.query_by_label("Settings…").is_none(),
        "no separate dialog"
    );
    let list_before = h.get_by_label("Remove All…").rect();
    h.get_by_label("Settings").click();
    h.run_steps(2);
    assert!(h.state().settings_open);
    assert_eq!(h.get_by_label("Remove All…").rect(), list_before);
    h.get_by_label_contains("Use Settings, then Config Save");
    // Connecting keeps the section open but locks it.
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    h.get_by_label_contains("Disconnect to edit");
    h.get_by_label("Settings").click();
    h.run_steps(2);
    assert!(!h.state().settings_open);
}

#[test]
fn app_stays_dark_when_the_system_theme_is_light() {
    // A light OS theme reaches egui as `RawInput::system_theme`; without a
    // pinned preference egui would switch to its unstyled light style.
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let input = || egui::RawInput {
        system_theme: Some(egui::Theme::Light),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(input(), |_| {});
    }
    assert_eq!(ctx.theme(), egui::Theme::Dark);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let style = ctx.style_of(theme);
        assert!(style.visuals.dark_mode, "{theme:?} slot is dark");
        assert_eq!(style.visuals.panel_fill, crate::theme::BACKGROUND);
        assert_eq!(style.visuals.widgets.inactive.bg_fill, crate::theme::BUTTON);
        assert_eq!(style.visuals.text_edit_bg_color, Some(crate::theme::INPUT));
    }
    // Control: without `configure`, the same input turns egui light.
    let plain = egui::Context::default();
    for _ in 0..3 {
        let _ = plain.run_ui(input(), |_| {});
    }
    assert_eq!(plain.theme(), egui::Theme::Light);
}

#[test]
fn startup_fit_folds_panels_and_caps_the_minimum() {
    let mut h = harness(true);
    h.set_size(egui::Vec2::new(1350., 672.));
    h.run_steps(1);
    // 1366 × 768 screen: maximize, log folded, minimum capped to the screen.
    let fit = crate::layout::fit_window(egui::Vec2::new(1366., 768.), egui::Vec2::new(1566., 900.));
    let ctx = h.ctx.clone();
    h.state_mut().apply_window_fit(&ctx, fit);
    h.run_steps(2);
    let app = h.state();
    assert!(app.window_fit.done && app.right_collapsed && !app.left_collapsed);
    assert_eq!(app.window_fit.limit, Some(egui::Vec2::new(1350., 672.)));
    // The fitted layout still offers the device controls.
    h.get_by_label("Connect");

    // Opening either panel on this screen makes room by folding the other.
    h.get_all_by_label("<").last().unwrap().click();
    h.run_steps(2);
    assert!(h.state().left_collapsed && !h.state().right_collapsed);
    assert!(h.query_by_label("Connect").is_none());
    h.get_all_by_label(">").next().unwrap().click();
    h.run_steps(2);
    assert!(!h.state().left_collapsed && h.state().right_collapsed);
    h.get_by_label("Connect");
}

fn pick_colormap(h: &mut Harness<'static, App>, from: &str, to: &str) {
    h.get_by_value(from).click();
    h.run_steps(2);
    h.get_all_by_label(to).last().unwrap().click();
    h.run_steps(2);
}

#[test]
fn colormap_selector_updates_preview_and_restores_from_config() {
    use slm_core::heatmap::Colormap;
    let mut h = harness(false);
    assert_eq!(h.state().config.preview_colormap, Colormap::Grayscale);
    h.state_mut().uniform_code = 700;
    h.run_steps(1);
    wait_preview(&mut h);
    let gray = h.state().preview.info.clone().unwrap();
    assert_eq!(gray.colormap, Colormap::Grayscale);

    // Switching is immediate: the old image no longer counts as current,
    // and the new one has the same codes and digest in other colors.
    pick_colormap(&mut h, "Grayscale", "Viridis");
    assert_eq!(h.state().config.preview_colormap, Colormap::Viridis);
    assert!(h.state().config_dirty);
    wait_preview(&mut h);
    let viridis = h.state().preview.info.clone().unwrap();
    assert_eq!(viridis.colormap, Colormap::Viridis);
    assert_eq!(viridis.digest, gray.digest);
    assert_eq!(
        (viridis.min_code, viridis.max_code_used, viridis.scale_max),
        (700, 700, 1023)
    );
    pick_colormap(&mut h, "Viridis", "Rainbow");
    wait_preview(&mut h);
    assert_eq!(
        h.state().preview.info.as_ref().unwrap().colormap,
        Colormap::Rainbow
    );
    pick_colormap(&mut h, "Rainbow", "Viridis");
    wait_preview(&mut h);

    // Config Save writes it; Load restores it.
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert!(!h.state().config_dirty);
    let path = h.state().config_path.clone();
    assert_eq!(
        slm_device::AppConfig::load(&path)
            .unwrap()
            .0
            .preview_colormap,
        Colormap::Viridis
    );
    pick_colormap(&mut h, "Viridis", "Grayscale");
    h.get_by_label("Load").click();
    h.run_steps(2);
    assert_eq!(h.state().config.preview_colormap, Colormap::Grayscale);
    h.get_by_label("Discard and Load").click();
    h.run_steps(2);
    assert_eq!(h.state().config.preview_colormap, Colormap::Viridis);
    wait_preview(&mut h);
    assert_eq!(
        h.state().preview.info.as_ref().unwrap().colormap,
        Colormap::Viridis
    );
}

#[test]
fn colormap_never_changes_what_is_sent() {
    use slm_core::heatmap::Colormap;
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    // Applied list row: preview it under each colormap.
    h.state_mut().uniform_code = 321;
    h.run_steps(1);
    h.get_by_label("Add to List").click();
    h.run_steps(1);
    h.get_by_label("Experiment").click();
    h.run_steps(1);
    h.state_mut().selected = Some(0);
    h.run_steps(1);
    h.get_by_label("Apply Row").click();
    until(&mut h, "row applied", |a| {
        a.pending.is_none() && matches!(a.status, ApplyStatus::Applied(..))
    });
    let sent = h.state().demo_control.lock().last_frame.clone().unwrap();
    let shows_before = shows(&h);
    for (from, to) in [("Grayscale", "Viridis"), ("Viridis", "Rainbow")] {
        pick_colormap(&mut h, from, to);
        wait_preview(&mut h);
        let info = h.state().preview.info.clone().unwrap();
        assert_eq!(info.digest, sent.digest(), "{to}: same codes as sent");
    }
    assert_eq!(h.state().config.preview_colormap, Colormap::Rainbow);
    assert_eq!(shows(&h), shows_before, "a colormap change never sends");
    assert_eq!(last_code(&h), Some(321));
}

#[test]
fn name_changes_are_dirty_and_close_can_be_cancelled() {
    let mut h = harness(true);
    h.get_by_label("Experiment").click();
    h.run_steps(2);
    h.get_all_by_value("Untitled").next().unwrap().click();
    h.run_steps(1);
    h.get_all_by_value("Untitled")
        .next()
        .unwrap()
        .type_text("changed");
    h.run_steps(2);
    assert!(h.state().experiment.dirty);
    assert!(!h.state_mut().request_close());
    h.run_steps(2);
    h.get_by_label("Save changes before closing?");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().experiment.dirty);
    h.get_by_label("New List").click();
    h.run_steps(2);
    h.get_by_label("Discard and Continue");
}

#[test]
fn failed_save_and_close_keeps_document_open_and_dirty() {
    let mut h = harness(true);
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("foreign.json");
    std::fs::write(&bad, "not a list").unwrap();
    h.state_mut().experiment.path = Some(bad.clone());
    h.state_mut().experiment.dirty = true;
    assert!(!h.state_mut().request_close());
    h.run_steps(2);
    h.get_by_label("Save and Close").click();
    h.run_steps(2);
    h.get_by_label("Save changes before closing?");
    assert!(h.state().experiment.dirty);
    assert_eq!(std::fs::read_to_string(bad).unwrap(), "not a list");
}

#[test]
fn config_failed_save_keeps_active_path_and_dirty_state() {
    let mut h = harness(false);
    let active = h.state().config_path.clone();
    let bad = active.with_file_name("foreign.json");
    std::fs::write(&bad, "{\"schema_version\":99}").unwrap();
    h.state_mut().config_dirty = true;
    assert!(!h.state_mut().save_config_to(&bad));
    assert_eq!(h.state().config_path, active);
    assert!(h.state().config_dirty);
    assert_eq!(
        std::fs::read_to_string(bad).unwrap(),
        "{\"schema_version\":99}"
    );
}

#[test]
fn export_runs_in_background_with_snapshot_and_closing_guard() {
    let mut h = harness(true);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("export");
    h.state_mut().config.profile.generation.raster = slm_core::Raster {
        width: 6,
        height: 4,
    };
    h.get_by_label("Add to List").click();
    h.run_steps(2);
    h.state_mut().export_experiment(&out);
    assert!(h.state().export_progress.is_some());
    assert!(!h.state_mut().request_close());
    h.state_mut().experiment.experiment.name = "changed after export".into();
    until(&mut h, "export completion", |a| a.export_progress.is_none());
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 1);
    assert_ne!(value["experiment_name"], "changed after export");
}

#[test]
#[cfg(target_os = "macos")]
fn mac_installed_fallback_can_draw_japanese_names() {
    let h = harness(true);
    assert!(
        h.ctx
            .fonts_mut(|fonts| fonts.has_glyphs(&egui::FontId::proportional(13.), "テスト日本語"))
    );
}

#[test]
fn close_waits_for_reset_and_failure_requires_explicit_choice() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    h.state().demo_control.lock().fail_next_show = Some("injected closing reset failure".into());
    assert!(!h.state_mut().request_close());
    until(&mut h, "reset failure", |a| {
        a.log
            .iter()
            .any(|l| l.text.contains("injected closing reset failure"))
    });
    assert!(!h.state_mut().request_close());
    h.run_steps(2);
    h.get_by_label("Close Anyway").click();
    h.run_steps(1);
    assert!(h.state_mut().request_close());
}

#[test]
fn successful_close_resets_before_allowing_exit() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    let before = shows(&h);
    assert!(!h.state_mut().request_close());
    until(&mut h, "shutdown receipt", |a| {
        a.log
            .iter()
            .any(|l| l.text.contains("Output session ended"))
    });
    assert_eq!(shows(&h), before + 1);
    assert_eq!(last_code(&h), Some(0));
    assert!(h.state_mut().request_close());
}

#[test]
fn undo_remove_restores_rows_without_losing_later_additions_or_applying() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    for code in [100, 200] {
        h.state_mut().uniform_code = code;
        h.get_by_label("Add to List").click();
        h.run_steps(2);
    }
    let removed_id = h.state().experiment.experiment.items[1].id.clone();
    let before = shows(&h);
    h.get_by_label("Remove Selected").click();
    h.run_steps(2);
    h.state_mut().uniform_code = 300;
    h.get_by_label("Add to List").click();
    h.run_steps(2);
    h.get_by_label("Undo Remove").click();
    h.run_steps(2);
    assert_eq!(h.state().experiment.experiment.items.len(), 3);
    assert_eq!(h.state().experiment.experiment.items[1].id, removed_id);
    assert_eq!(shows(&h), before);
    assert!(h.state().experiment.dirty);
}

#[test]
fn single_save_is_async_and_uses_click_time_snapshot() {
    let mut h = harness(true);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("snapshot.csv");
    h.state_mut().config.profile.generation.raster = slm_core::Raster {
        width: 6,
        height: 4,
    };
    h.state_mut().uniform_code = 123;
    h.state_mut().save_preview_to(&out, false);
    assert!(h.state().export_progress.is_some());
    assert!(!h.state_mut().request_close());
    h.state_mut().uniform_code = 456;
    until(&mut h, "single save completion", |a| {
        a.export_progress.is_none()
    });
    let text = std::fs::read_to_string(out).unwrap();
    assert!(text.contains("123"));
    assert!(!text.contains("456"));
    h.state_mut().save_preview_to(dir.path(), false);
    until(&mut h, "save failure", |a| a.export_progress.is_none());
    assert!(
        h.state()
            .log
            .iter()
            .any(|l| l.level == crate::app::Level::Error)
    );
}

#[test]
fn settings_are_drafts_until_use_and_cancel_discards() {
    let mut h = harness(true);
    let old = h.state().config.profile.generation.optics.pixel_pitch_um;
    h.get_by_label("Settings").click();
    h.run_steps(2);
    h.get_all_by_value(&old.to_string()).next().unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_all_by_value(&old.to_string())
        .next()
        .unwrap()
        .type_text("20");
    h.run_steps(2);
    assert_eq!(
        h.state().config.profile.generation.optics.pixel_pitch_um,
        old
    );
    h.get_by_label("Cancel Settings").click();
    h.run_steps(2);
    assert_eq!(
        h.state().config.profile.generation.optics.pixel_pitch_um,
        old
    );
    assert!(!h.state().settings_open);
}

#[test]
fn settings_use_commits_draft_without_sending() {
    let mut h = harness(true);
    let old = h.state().config.profile.generation.optics.pixel_pitch_um;
    h.get_by_label("Settings").click();
    h.run_steps(2);
    h.get_all_by_value(&old.to_string()).next().unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_all_by_value(&old.to_string())
        .next()
        .unwrap()
        .type_text("20");
    h.run_steps(2);
    h.get_by_label("Use Settings").click();
    h.run_steps(2);
    assert_eq!(
        h.state().config.profile.generation.optics.pixel_pitch_um,
        20.
    );
    assert!(h.state().config_dirty);
    assert_eq!(shows(&h), 0);
}

#[test]
fn list_shortcuts_apply_save_and_undo_but_not_while_typing() {
    let mut h = harness(true);
    let dir = tempfile::tempdir().unwrap();
    h.state_mut().experiment.path = Some(dir.path().join("list.json"));
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    h.get_by_label("Auto-apply on click").click();
    h.state_mut().uniform_code = 12;
    h.get_by_label("Add to List").click();
    h.run_steps(2);
    h.get_by_label("#1 Uniform code 12").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    until(&mut h, "keyboard apply", |a| a.showing_index() == Some(0));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(2);
    assert!(!h.state().experiment.dirty);
    assert!(dir.path().join("list.json").is_file());
    h.get_by_label("Remove Selected").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().experiment.experiment.items.len(), 1);
    h.get_by_label("Experiment").click();
    h.run_steps(2);
    h.get_all_by_value("Untitled").next().unwrap().click();
    h.run_steps(1);
    let before = shows(&h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(shows(&h), before, "typing cannot trigger apply");
}

#[test]
fn compact_numeric_display_and_invalid_text_preserve_full_value() {
    let mut h = harness(true);
    let exact = 123.1234567890123;
    h.state_mut().single.wave.period.value = exact;
    h.get_by_label("Single Wave").click();
    h.run_steps(2);
    assert_eq!(h.state().single.wave.period.value, exact);
    h.get_all_by_value("1.2312e2").next().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().single.wave.period.value, exact);
    h.get_all_by_value(&exact.to_string())
        .next()
        .expect("focused numeric field shows full round-trip precision");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("NaN".into()));
    h.run_steps(2);
    assert_eq!(h.state().single.wave.period.value, exact);
}

#[test]
fn closing_with_settings_draft_requires_explicit_resolution() {
    let mut h = harness(true);
    let old = h.state().config.profile.generation.optics.pixel_pitch_um;
    h.get_by_label("Settings").click();
    h.run_steps(2);
    h.get_all_by_value(&old.to_string()).next().unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_all_by_value(&old.to_string())
        .next()
        .unwrap()
        .type_text("20");
    h.run_steps(2);
    assert!(!h.state_mut().request_close());
    h.run_steps(2);
    assert!(h.state().settings_open);
    assert_eq!(
        h.state().config.profile.generation.optics.pixel_pitch_um,
        old
    );
    h.get_by_label("Cancel Settings").click();
    h.run_steps(2);
    assert!(!h.state().settings_open);
    assert!(!h.state_mut().request_close()); // async shutdown begins only now
    until(&mut h, "clean shutdown", |a| {
        a.log
            .iter()
            .any(|l| l.text.contains("Already disconnected"))
    });
    assert!(h.state_mut().request_close());
}
