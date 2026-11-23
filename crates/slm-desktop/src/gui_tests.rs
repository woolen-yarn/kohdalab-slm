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
                    gui_api: false,
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
    for expected in [0u16, 10, 20] {
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
    h.get_by_label("#1 0 code").click();
    h.run_steps(2);
    assert_eq!(last_code(&h), Some(10));
    h.get_by_label("Selected · Row #1 · 0 code");
    h.get_by_label("Applied · Row #2 · 10 code");
    h.get_by_label("Save target · Row #1 · 0 code");
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
    let profile = slm_device::DeviceProfile::santec_slm200_candidate().generation;

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
    h.get_by_label("Save target · Row #3 · 20 code");
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
    h.get_by_label_contains("Saved by Config Save");
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

/// Sends one request on a background thread while the GUI keeps stepping,
/// as a real client would; returns the single JSON Lines reply.
fn query(h: &mut Harness<'static, App>, body: serde_json::Value) -> serde_json::Value {
    use std::io::{BufRead, BufReader, Write};
    let descriptor: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&h.state().control_api.as_ref().unwrap().descriptor).unwrap(),
    )
    .unwrap();
    let mut body = body;
    body["token"] = descriptor["token"].clone();
    let port = descriptor["port"].as_u64().unwrap() as u16;
    let client = std::thread::spawn(move || {
        let mut stream =
            std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        writeln!(stream, "{body}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.is_finished() {
        assert!(Instant::now() < deadline, "no reply");
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(5));
    }
    client.join().unwrap()
}

/// Tests inject a server in a temporary run directory, never `~/.kohdalab`.
fn with_gui_api(h: &mut Harness<'static, App>, dir: &std::path::Path) {
    let ctx = h.ctx.clone();
    h.state_mut().control_api = Some(
        crate::control_api::Server::start(Some(dir.into()), move || ctx.request_repaint()).unwrap(),
    );
    h.run_steps(1);
}

#[test]
fn standalone_launch_has_no_external_control() {
    let mut h = harness(true);
    h.run_steps(2);
    assert!(h.state().control_api.is_none());
    assert!(h.query_by_label(crate::app::EXTERNAL_CONTROL).is_none());
}

#[test]
fn read_only_api_observes_demo_state_and_never_drives_the_device() {
    use serde_json::json;
    let run = tempfile::tempdir().unwrap();
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    with_gui_api(&mut h, run.path());
    // Restrained indicator, and no take-control / stop affordance.
    h.get_by_label(crate::app::EXTERNAL_CONTROL);
    assert!(h.query_by_label("Take control").is_none());

    let status = query(
        &mut h,
        json!({"control_protocol":1,"method":"control.status","request_id":"s1"}),
    );
    assert_eq!(status["ok"], true);
    assert_eq!(status["request_id"], "s1");
    assert_eq!(status["result"]["phase"], "disconnected");
    assert_eq!(status["result"]["native"]["demo"], true);
    assert_eq!(status["result"]["native"]["simulated"], true);
    assert_eq!(status["result"]["ownership"]["owned"], false);

    // Write-shaped requests are refused and change nothing.
    for method in [
        "control.connect",
        "control.apply",
        "control.acquire",
        "control.stop",
    ] {
        let r = query(&mut h, json!({"control_protocol":1,"method":method}));
        assert_eq!(r["error"]["code"], "unsupported", "{method}");
    }
    h.run_steps(3);
    assert!(matches!(h.state().device_state, DeviceState::Disconnected));
    assert_eq!(shows(&h), 0, "the API never shows a frame");

    let config = query(
        &mut h,
        json!({"control_protocol":1,"method":"control.config"}),
    );
    assert_eq!(config["result"]["source"], "temporary_demo");
    assert_eq!(config["result"]["output"]["backend"], "demo");
    assert!(!config.to_string().contains("config.json"));

    // The GUI keeps full control; status follows what the GUI did.
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    let status = query(
        &mut h,
        json!({"control_protocol":1,"method":"control.status"}),
    );
    assert_eq!(status["result"]["phase"], "ready");
    assert_eq!(status["result"]["native"]["device"], "connected");
    assert_eq!(
        status["result"]["native"]["showing"]["receipt"]["verification"],
        "Simulated"
    );
    assert_eq!(
        shows(&h),
        1,
        "only the GUI's own connect showed the reset frame"
    );

    let descriptor = h.state().control_api.as_ref().unwrap().descriptor.clone();
    drop(h);
    assert!(
        !descriptor.exists(),
        "descriptor removed when the app closes"
    );
}
