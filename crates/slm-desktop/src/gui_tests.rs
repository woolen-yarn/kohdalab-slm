//! Headless GUI checks on the real `App` (demo backend, temporary config).
//! No window, display or device is opened.

use crate::app::{App, AppOptions, ApplyStatus, Tab};
use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};
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
    assert!(
        h.query_by_label_contains(crate::app::EXTERNAL_CONTROL)
            .is_none()
    );
    assert!(h.query_by_label("Take control").is_none());
}

#[test]
fn api_reads_state_and_refuses_unleased_or_malformed_mutations() {
    use serde_json::json;
    let run = tempfile::tempdir().unwrap();
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    with_gui_api(&mut h, run.path());
    // Restrained indicator with Stop and Take control (both idle).
    h.get_by_label_contains("External control · Available");
    h.get_by_label("Take control");
    h.get_by_label("Stop");

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
    assert_eq!(status["result"]["ownership"]["supported"], true);

    // Over the real socket: no lease, no request ID, or an unknown method
    // is refused and changes nothing.
    let rid = || uuid::Uuid::new_v4().to_string();
    for (body, code) in [
        (
            json!({"control_protocol":1,"method":"control.connect","client_id":"c","request_id":rid()}),
            "ownership",
        ),
        (
            json!({"control_protocol":1,"method":"control.apply","client_id":"c","request_id":rid(),"spec":{"kind":"uniform","code":1}}),
            "ownership",
        ),
        (
            json!({"control_protocol":1,"method":"control.stop","client_id":"c","request_id":rid(),"lease":"forged"}),
            "ownership",
        ),
        (
            json!({"control_protocol":1,"method":"control.acquire","client_id":"c"}),
            "validation",
        ),
        (
            json!({"control_protocol":1,"method":"control.acquire","client_id":"c","request_id":"not-a-uuid"}),
            "validation",
        ),
        (
            json!({"control_protocol":1,"method":"control.acquire","client_id":"c","request_id":rid(),"display":"\\\\.\\DISPLAY1"}),
            "validation",
        ),
        (
            json!({"control_protocol":1,"method":"control.start","client_id":"c","request_id":rid()}),
            "unsupported",
        ),
        (
            json!({"control_protocol":1,"method":"control.result","job_id":"nope"}),
            "unknown_job",
        ),
    ] {
        let r = query(&mut h, body.clone());
        assert_eq!(r["ok"], false, "{body}");
        assert_eq!(r["error"]["code"], code, "{body}: {r}");
        assert_eq!(r["error"]["retryable"], false);
    }
    h.run_steps(3);
    assert!(matches!(h.state().device_state, DeviceState::Disconnected));
    assert!(h.state().control.lease.is_none());
    assert_eq!(shows(&h), 0, "no frame was shown");

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

    // Disconnect is in flight until the worker reports it: busy, output
    // pending, and the prior receipt is not presented as current.
    h.state_mut().disconnect();
    assert!(h.state().disconnecting);
    let snapshot = crate::control_api::status(&h.state().control_snapshot());
    assert_eq!(snapshot["phase"], "stopping");
    assert_eq!(snapshot["busy"], true);
    assert_eq!(snapshot["cleanup"], "pending");
    assert_eq!(snapshot["native"]["showing"], serde_json::Value::Null);
    until(&mut h, "disconnected", |a| {
        matches!(a.device_state, DeviceState::Disconnected) && !a.disconnecting
    });
    let status = query(
        &mut h,
        json!({"control_protocol":1,"method":"control.status"}),
    );
    assert_eq!(status["result"]["busy"], false);
    assert_eq!(status["result"]["cleanup"], "not_required");

    let descriptor = h.state().control_api.as_ref().unwrap().descriptor.clone();
    drop(h);
    assert!(
        !descriptor.exists(),
        "descriptor removed when the app closes"
    );
}

#[test]
fn ended_listener_revokes_the_lease_and_stops_output() {
    let run = tempfile::tempdir().unwrap();
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    with_gui_api(&mut h, run.path());
    h.get_by_label_contains(crate::app::EXTERNAL_CONTROL);
    let mut py = Client::new();
    assert_eq!(mutate(&mut h, &mut py, "acquire", json!({}))["ok"], true);
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_job(&mut h, &job);
    let job = mutate(
        &mut h,
        &mut py,
        "apply",
        json!({"spec": {"kind": "uniform", "code": 9}}),
    );
    wait_job(&mut h, &job);
    h.get_by_label(crate::app::EXTERNAL_LEASED);

    let descriptor = h.state().control_api.as_ref().unwrap().descriptor.clone();
    h.state().control_api.as_ref().unwrap().stop_listener();
    until(&mut h, "server dropped", |a| a.control_api.is_none());
    // Transport loss: lease revoked and the reset/close path started.
    assert!(h.state().control.lease.is_none());
    until(&mut h, "stop acknowledged", |a| {
        a.control.running().is_none() && !a.connected()
    });
    let s = crate::control_api::status(&h.state().control_snapshot());
    assert_eq!(s["ownership"]["last_revocation"], "listener_ended");
    assert_eq!(s["cleanup"], "not_required");
    assert_eq!(last_code(&h), Some(0), "reset frame shown before closing");
    assert!(!h.state().demo_control.lock().open);
    h.run_steps(2);
    assert!(h.query_by_label(crate::app::EXTERNAL_LEASED).is_none());
    assert!(!descriptor.exists(), "descriptor withdrawn");
}

#[test]
fn unknown_output_survives_disconnect_until_a_confirmed_connect() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    h.get_by_label("Connect").click();
    until(&mut h, "connected", |a| a.connected());
    // Invalidate the demo output, as a moved or lost display would.
    h.state().demo_control.lock().pending_invalidation = Some("Display moved.".into());
    until(&mut h, "output unknown", |a| !a.connected());
    h.state_mut().disconnect();
    until(&mut h, "disconnected", |a| {
        matches!(a.device_state, DeviceState::Disconnected) && !a.disconnecting
    });
    let s = crate::control_api::status(&h.state().control_snapshot());
    assert_eq!(
        s["cleanup"], "unknown",
        "closing does not verify the output"
    );
    assert_eq!(s["native"]["output"], "unknown");
    h.state_mut().connect();
    until(&mut h, "reconnected", |a| a.connected());
    let s = crate::control_api::status(&h.state().control_snapshot());
    assert_eq!(s["cleanup"], "not_required");
    assert_eq!(s["native"]["output"], "confirmed");
}

// ----- leased external control (demo backend only) -----------------------

/// A client as the Python SDK behaves: random client ID, fresh UUID request
/// IDs for mutations, and the lease attached once acquired.
struct Client {
    id: String,
    lease: Option<String>,
}

impl Client {
    fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            lease: None,
        }
    }
}

/// One request answered on the GUI thread exactly as the server queues it
/// (deterministic: no socket, no timing), then one frame.
fn call(h: &mut Harness<'static, App>, method: &str, mut body: Value) -> Value {
    body["control_protocol"] = json!(1);
    body["method"] = json!(format!("control.{method}"));
    // Stands in for the SDK heartbeat thread between direct calls; expiry is
    // tested by aging the lease explicitly between frames.
    if let Some(lease) = h.state_mut().control.lease.as_mut() {
        lease.last_seen = Instant::now();
    }
    let reply = h.state_mut().answer_control(&body);
    h.run_steps(1);
    reply
}

fn mutate_id(
    h: &mut Harness<'static, App>,
    c: &mut Client,
    method: &str,
    mut params: Value,
    request_id: &str,
) -> Value {
    params["client_id"] = json!(c.id);
    params["request_id"] = json!(request_id);
    if let Some(lease) = &c.lease {
        params["lease"] = json!(lease);
    }
    let reply = call(h, method, params);
    assert_eq!(reply["request_id"], request_id);
    if method == "acquire" && reply["ok"] == true {
        c.lease = reply["result"]["lease"].as_str().map(String::from);
    }
    reply
}

fn mutate(h: &mut Harness<'static, App>, c: &mut Client, method: &str, params: Value) -> Value {
    let id = uuid::Uuid::new_v4().to_string();
    mutate_id(h, c, method, params, &id)
}

fn receipt(h: &mut Harness<'static, App>, job_id: &str) -> Value {
    let r = call(h, "result", json!({"job_id": job_id}));
    assert_eq!(r["ok"], true, "{r}");
    r["result"].clone()
}

/// Poll `control.result` until the job is terminal (worker acknowledged).
fn wait_job(h: &mut Harness<'static, App>, accepted: &Value) -> Value {
    wait_for_receipt(h, accepted, None)
}

fn wait_for_receipt(
    h: &mut Harness<'static, App>,
    accepted: &Value,
    mut owner: Option<&mut Client>,
) -> Value {
    assert_eq!(accepted["ok"], true, "not accepted: {accepted}");
    let id = accepted["result"]["job_id"]
        .as_str()
        .expect("job receipt")
        .to_string();
    let mut heartbeat_due = Instant::now();
    let deadline = heartbeat_due + Duration::from_secs(10);
    loop {
        if Instant::now() >= heartbeat_due
            && let Some(client) = owner.as_deref_mut()
        {
            let heartbeat = mutate(h, client, "heartbeat", json!({}));
            assert_eq!(heartbeat["ok"], true, "{heartbeat}");
            heartbeat_due = Instant::now() + Duration::from_millis(500);
        }
        let r = receipt(h, &id);
        assert_eq!(r["job_id"], id.as_str());
        if r["terminal"] == true {
            return r;
        }
        assert!(Instant::now() < deadline, "job {id} never finished: {r}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn api_status(h: &Harness<'static, App>) -> Value {
    crate::control_api::status(&h.state().control_snapshot())
}

fn uniform(code: u16) -> Value {
    json!({"spec": {"kind": "uniform", "code": code}})
}

#[test]
fn leased_connect_apply_reset_stop_release_complete_on_worker_receipts() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    let mut py = Client::new();
    let r = mutate(&mut h, &mut py, "connect", json!({}));
    assert_eq!(r["error"]["code"], "ownership", "no lease: {r}");
    let r = mutate(&mut h, &mut py, "acquire", json!({}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["heartbeat_seconds"], 0.5);
    assert_eq!(r["result"]["on_release"], "stop");
    assert_eq!(api_status(&h)["ownership"]["owned"], true);
    assert!(
        !api_status(&h)
            .to_string()
            .contains(py.lease.as_deref().unwrap())
    );

    // Connect: the GUI's display and confirmed mode; the ACK is not done.
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    assert_eq!(job["result"]["status"], "running", "{job}");
    assert_eq!(job["result"]["terminal"], false);
    assert_eq!(job["result"]["cleanup"], "pending");
    let r = wait_for_receipt(&mut h, &job, Some(&mut py));
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["native"]["receipt"]["uniform_code"], 0);
    assert_eq!(r["native"]["receipt"]["verification"], "Simulated");
    assert_eq!(r["native"]["mode"], "60hz");
    assert_eq!(r["native"]["display"]["friendly_name"], "SLM-200 (demo)");
    assert!(h.state().connected());
    assert_eq!(shows(&h), 1);

    // Apply a validated PatternSpec; the GUI shows it like its own apply.
    let job = mutate(
        &mut h,
        &mut py,
        "apply",
        json!({"spec": {"kind": "uniform", "code": 512}, "label": "Python 512"}),
    );
    let r = wait_for_receipt(&mut h, &job, Some(&mut py));
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["native"]["receipt"]["uniform_code"], 512);
    assert_eq!(r["native"]["label"], "Python 512");
    assert_eq!(last_code(&h), Some(512));
    assert!(matches!(h.state().status, ApplyStatus::Applied(..)));

    // Apply an existing Patterns row by its item ID.
    h.state_mut()
        .add_to_experiment("Row A".into(), slm_core::PatternSpec::Uniform { code: 77 });
    let id = h.state().experiment.experiment.items[0].id.clone();
    let job = mutate(&mut h, &mut py, "apply", json!({"item_id": id}));
    let r = wait_for_receipt(&mut h, &job, Some(&mut py));
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["native"]["source"], "item");
    assert_eq!(r["native"]["receipt"]["uniform_code"], 77);
    assert_eq!(h.state().showing_index(), Some(0));

    // Reset: the profile's uniform raw code, stated as such.
    let job = mutate(&mut h, &mut py, "reset", json!({}));
    let r = wait_for_receipt(&mut h, &job, Some(&mut py));
    assert_eq!(r["status"], "completed");
    assert_eq!(r["native"]["reset_code"], 0);
    assert!(
        r["native"]["reset_meaning"]
            .as_str()
            .unwrap()
            .contains("not verified dark")
    );
    assert_eq!(last_code(&h), Some(0));

    // Stop: worker reset then close; the lease is kept.
    let job = mutate(&mut h, &mut py, "stop", json!({}));
    let r = wait_for_receipt(&mut h, &job, Some(&mut py));
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["cleanup"], "verified");
    assert_eq!(r["native"]["reset"]["outcome"], "confirmed");
    assert_eq!(r["native"]["reset"]["receipt"]["uniform_code"], 0);
    assert_eq!(r["native"]["optical_state"], "not_verified");
    assert_eq!(r["native"]["output_closed"], true);
    assert!(!h.state().demo_control.lock().open);
    assert!(matches!(h.state().device_state, DeviceState::Disconnected));
    assert_eq!(api_status(&h)["ownership"]["owned"], true);

    // Reconnect and disconnect under the same lease.
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_for_receipt(&mut h, &job, Some(&mut py));
    let job = mutate(&mut h, &mut py, "disconnect", json!({}));
    let r = wait_for_receipt(&mut h, &job, Some(&mut py));
    assert_eq!(r["kind"], "disconnect");
    assert_eq!(r["cleanup"], "verified");

    // Release always stops; with nothing open the stop is a known no-op.
    let r = mutate(&mut h, &mut py, "release", json!({}));
    assert_eq!(r["result"]["released"], true, "{r}");
    assert_eq!(r["result"]["on_release"], "stop");
    let job_id = r["result"]["job_id"].as_str().unwrap().to_string();
    let r = wait_job(&mut h, &json!({"ok": true, "result": {"job_id": job_id}}));
    assert_eq!(r["status"], "completed");
    assert_eq!(r["cleanup"], "not_required");
    assert_eq!(r["native"]["origin"], "release");
    let s = api_status(&h);
    assert_eq!(s["ownership"]["owned"], false);
    assert_eq!(s["ownership"]["last_revocation"], "released");
    assert_eq!(s["busy"], false);

    // The old lease is revoked, not silently accepted.
    let r = mutate(&mut h, &mut py, "connect", json!({}));
    assert_eq!(r["error"]["code"], "ownership");
    assert_eq!(r["error"]["application_code"], "revoked");
    assert!(matches!(h.state().device_state, DeviceState::Disconnected));
}

#[test]
fn operations_are_serial_deduplicated_and_strictly_validated() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    let mut py = Client::new();
    mutate(&mut h, &mut py, "acquire", json!({}));
    // Not connected: nothing is sent.
    let r = mutate(&mut h, &mut py, "apply", uniform(1));
    assert_eq!(r["error"]["application_code"], "not_connected", "{r}");
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_job(&mut h, &job);

    for (params, why) in [
        (
            json!({"spec": {"kind": "uniform", "code": 5000}}),
            "outside",
        ),
        (
            json!({"spec": {"kind": "uniform", "code": 1, "extra": 0}}),
            "spec.extra",
        ),
        (
            json!({"spec": {"kind": "raw_frame"}}),
            "Invalid PatternSpec",
        ),
        (json!({}), "exactly one"),
        (
            json!({"spec": {"kind": "uniform", "code": 1}, "item_id": "x"}),
            "exactly one",
        ),
        (json!({"item_id": "missing"}), "No Patterns row"),
        (
            json!({"spec": {"kind": "uniform", "code": 1}, "display": 2}),
            "Unknown parameter",
        ),
        (
            json!({"spec": {"kind": "uniform", "code": 1}, "label": ""}),
            "label",
        ),
    ] {
        let r = mutate(&mut h, &mut py, "apply", params.clone());
        assert_eq!(r["error"]["code"], "validation", "{params}: {r}");
        let message = r["error"]["message"].as_str().unwrap();
        assert!(message.contains(why), "{params}: {message}");
    }
    for method in ["connect", "reset", "disconnect", "stop", "heartbeat"] {
        let r = mutate(&mut h, &mut py, method, json!({"mode": "120hz"}));
        assert_eq!(r["error"]["code"], "validation", "{method}: {r}");
    }
    let r = mutate(&mut h, &mut py, "connect", json!({}));
    assert_eq!(r["error"]["application_code"], "state", "already connected");
    let before = shows(&h);

    // Hold the simulated backend so one apply stays in flight.
    let control = h.state().demo_control.clone();
    let hold = control.lock();
    let rid = uuid::Uuid::new_v4().to_string();
    let first = mutate_id(&mut h, &mut py, "apply", uniform(100), &rid);
    assert_eq!(first["ok"], true, "{first}");
    // Same request ID and request: the original answer, not a second apply.
    let again = mutate_id(&mut h, &mut py, "apply", uniform(100), &rid);
    assert_eq!(again["result"]["job_id"], first["result"]["job_id"]);
    assert_eq!(again["result"]["duplicate"], true);
    // Same ID, different request: refused.
    let r = mutate_id(&mut h, &mut py, "apply", uniform(101), &rid);
    assert_eq!(r["error"]["code"], "request_id_conflict", "{r}");
    // Serial: everything else waits for the receipt...
    for (method, params) in [
        ("apply", uniform(200)),
        ("reset", json!({})),
        ("disconnect", json!({})),
        ("connect", json!({})),
    ] {
        let r = mutate(&mut h, &mut py, method, params);
        assert_eq!(r["error"]["code"], "busy", "{method}: {r}");
    }
    // ...except Stop, which is queued behind the apply in the worker.
    let stop = mutate(&mut h, &mut py, "stop", json!({}));
    assert_eq!(stop["result"]["status"], "running", "{stop}");
    let s = api_status(&h);
    assert_eq!(s["busy"], true);
    assert_eq!(s["cleanup"], "pending");
    assert_eq!(s["native"]["output"], "pending");
    drop(hold);
    let applied = wait_job(&mut h, &first);
    assert_eq!(applied["status"], "completed");
    assert_eq!(applied["native"]["receipt"]["uniform_code"], 100);
    let stopped = wait_job(&mut h, &stop);
    assert_eq!(stopped["cleanup"], "verified");
    assert_eq!(shows(&h), before + 2, "one apply and one reset frame");
    assert_eq!(last_code(&h), Some(0));
    // A duplicate after completion still names the same job.
    let again = mutate_id(&mut h, &mut py, "apply", uniform(100), &rid);
    assert_eq!(again["result"]["job_id"], first["result"]["job_id"]);
    assert_eq!(shows(&h), before + 2);
}

#[test]
fn lease_expiry_stops_output_and_blocks_takeover_until_cleanup() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    let mut py = Client::new();
    mutate(&mut h, &mut py, "acquire", json!({}));
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_job(&mut h, &job);
    let job = mutate(&mut h, &mut py, "apply", uniform(300));
    wait_job(&mut h, &job);
    // A heartbeat renews the lease.
    let r = mutate(&mut h, &mut py, "heartbeat", json!({}));
    assert_eq!(r["result"]["renewed"], true);

    // Missed heartbeats, made deterministic by aging the last one.
    let control = h.state().demo_control.clone();
    let hold = control.lock();
    {
        let lease = h.state_mut().control.lease.as_mut().unwrap();
        lease.last_seen = lease
            .last_seen
            .checked_sub(Duration::from_secs(10))
            .unwrap();
    }
    h.run_steps(1);
    assert!(h.state().control.lease.is_none(), "expired lease revoked");
    let s = api_status(&h);
    assert_eq!(s["ownership"]["owned"], false);
    assert_eq!(s["ownership"]["last_revocation"], "expired");
    assert_eq!(s["busy"], true);
    assert_eq!(s["cleanup"], "pending");
    assert_eq!(s["native"]["operation"]["kind"], "stop");
    assert_eq!(s["native"]["external_control"], "cleanup_pending");
    let stop_id = s["native"]["operation"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // No takeover while the stop is unacknowledged; the stale lease is
    // revoked for every operation, including heartbeat and release.
    let mut other = Client::new();
    let r = mutate(&mut h, &mut other, "acquire", json!({}));
    assert_eq!(r["error"]["code"], "busy", "{r}");
    for method in ["heartbeat", "apply", "release", "stop"] {
        let r = mutate(&mut h, &mut py, method, uniform(1));
        assert_eq!(r["error"]["application_code"], "revoked", "{method}: {r}");
    }
    // Manual controls stay locked until the worker acknowledges.
    h.state_mut().reset_pattern();
    assert!(h.state().pending.is_none(), "manual reset refused");

    drop(hold);
    let r = wait_job(&mut h, &json!({"ok": true, "result": {"job_id": stop_id}}));
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["cleanup"], "verified");
    assert_eq!(r["native"]["origin"], "lease_expired");
    assert_eq!(last_code(&h), Some(0));
    assert!(!h.state().demo_control.lock().open);
    let s = api_status(&h);
    assert_eq!(s["busy"], false);
    assert_eq!(s["native"]["external_control"], "available");
    // Cleanup confirmed: a new client may now acquire.
    let r = mutate(&mut h, &mut other, "acquire", json!({}));
    assert_eq!(r["ok"], true, "{r}");
}

#[test]
fn take_control_and_manual_stop_end_the_lease_and_lock_manual_controls() {
    let run = tempfile::tempdir().unwrap();
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    with_gui_api(&mut h, run.path());
    h.state_mut()
        .add_to_experiment("Row X".into(), slm_core::PatternSpec::Uniform { code: 44 });
    let mut py = Client::new();
    mutate(&mut h, &mut py, "acquire", json!({}));
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_job(&mut h, &job);
    h.run_steps(2);
    h.get_by_label(crate::app::EXTERNAL_LEASED);
    let before = shows(&h);

    // Competing manual mutations are refused while leased: buttons are
    // disabled, auto-apply clicks (on in the demo) only select, and direct
    // calls are refused with a log line.
    for label in ["Disconnect", "Reset Pattern", "Apply", "Connect"] {
        h.get_by_label(label).click();
        h.run_steps(2);
    }
    h.get_by_label("#1 Row X").click();
    h.run_steps(3);
    h.state_mut().disconnect();
    h.state_mut()
        .apply_spec("Manual".into(), slm_core::PatternSpec::Uniform { code: 3 });
    h.run_steps(3);
    assert_eq!(shows(&h), before, "nothing manual was sent");
    assert!(h.state().connected());
    assert!(h.state().pending.is_none());
    assert!(
        h.state()
            .log
            .iter()
            .any(|l| l.text.contains("External control holds the SLM"))
    );

    // File dialogs would block lease expiry and Stop on the GUI thread.
    h.get_by_label("Experiment").click();
    h.run_steps(2);
    use egui_kittest::kittest::NodeT;
    for label in [
        "Open List…",
        "Save List",
        "Save List As…",
        "Import CSV…",
        "Export…",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    assert!(!h.get_by_label("Stop").accesskit_node().is_disabled());
    // Take control: lease ends, output unchanged, old lease revoked.
    h.get_by_label("Take control").click();
    h.run_steps(2);
    assert!(h.state().control.lease.is_none());
    assert!(h.state().connected());
    assert_eq!(shows(&h), before);
    assert_eq!(
        api_status(&h)["ownership"]["last_revocation"],
        "gui_take_control"
    );
    let r = mutate(&mut h, &mut py, "reset", json!({}));
    assert_eq!(r["error"]["application_code"], "revoked");
    // Manual control is back.
    h.get_by_label("Reset Pattern").click();
    until(&mut h, "manual reset", |a| {
        a.pending.is_none() && matches!(a.status, ApplyStatus::Applied(..))
    });
    assert_eq!(shows(&h), before + 1);

    // A new client takes the idle, verified output; the GUI Stop ends it.
    let mut py2 = Client::new();
    assert_eq!(mutate(&mut h, &mut py2, "acquire", json!({}))["ok"], true);
    h.run_steps(2);
    h.get_by_label("Stop").click();
    until(&mut h, "manual stop acknowledged", |a| {
        a.control.running().is_none() && !a.connected()
    });
    assert!(h.state().control.lease.is_none());
    let s = api_status(&h);
    assert_eq!(s["ownership"]["last_revocation"], "gui_stop");
    assert_eq!(s["cleanup"], "not_required");
    assert_eq!(last_code(&h), Some(0));
    assert!(!h.state().demo_control.lock().open);
    let r = mutate(&mut h, &mut py2, "connect", json!({}));
    assert_eq!(r["error"]["application_code"], "revoked");
    h.run_steps(2);
    h.get_by_label_contains("External control · Available");
}

#[test]
fn unknown_output_fails_receipts_and_blocks_takeover_until_gui_reconnect() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    let mut py = Client::new();
    mutate(&mut h, &mut py, "acquire", json!({}));
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_job(&mut h, &job);
    h.state().demo_control.lock().fail_next_show = Some("Simulated draw failure".into());
    let job = mutate(&mut h, &mut py, "apply", uniform(5));
    let r = wait_job(&mut h, &job);
    assert_eq!(r["status"], "error", "{r}");
    assert_eq!(r["cleanup"], "unknown");
    assert_eq!(r["native"]["output_known"], false);
    let s = api_status(&h);
    assert_eq!(s["phase"], "error");
    assert_eq!(s["cleanup"], "unknown");

    // Stop cannot verify an unknown output: no reset was possible.
    let job = mutate(&mut h, &mut py, "stop", json!({}));
    let r = wait_job(&mut h, &job);
    assert_eq!(r["status"], "error", "{r}");
    assert_eq!(r["cleanup"], "unknown");
    assert_eq!(r["native"]["reset"]["outcome"], "not_open");
    assert_eq!(r["native"]["reset"]["previous_output_unknown"], true);
    let draws = shows(&h);
    let r = mutate(&mut h, &mut py, "connect", json!({}));
    assert_eq!(r["error"]["code"], "cleanup_unconfirmed", "{r}");
    assert_eq!(shows(&h), draws);
    assert!(h.state().output_unverified.is_some());
    // Release (always a stop) reports the same, never success.
    let r = mutate(&mut h, &mut py, "release", json!({}));
    let job_id = r["result"]["job_id"].as_str().unwrap().to_string();
    let r = wait_job(&mut h, &json!({"ok": true, "result": {"job_id": job_id}}));
    assert_eq!(r["status"], "error");
    assert_eq!(r["cleanup"], "unknown");
    let s = api_status(&h);
    assert_eq!(s["phase"], "error");
    assert_eq!(s["cleanup"], "unknown");
    assert_eq!(s["native"]["external_control"], "output_unverified");

    // No takeover of an unverified output.
    let mut other = Client::new();
    let r = mutate(&mut h, &mut other, "acquire", json!({}));
    assert_eq!(r["error"]["code"], "cleanup_unconfirmed", "{r}");
    // Only a backend-confirmed GUI connect clears it.
    h.state_mut().connect();
    until(&mut h, "reconnected", |a| a.connected());
    h.state_mut().disconnect();
    until(&mut h, "disconnected", |a| {
        matches!(a.device_state, DeviceState::Disconnected) && !a.disconnecting
    });
    let r = mutate(&mut h, &mut other, "acquire", json!({}));
    assert_eq!(r["ok"], true, "{r}");
}

#[test]
fn failed_reset_before_close_is_not_reported_as_cleanup() {
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    let mut py = Client::new();
    mutate(&mut h, &mut py, "acquire", json!({}));
    let job = mutate(&mut h, &mut py, "connect", json!({}));
    wait_job(&mut h, &job);
    h.state().demo_control.lock().fail_next_show = Some("Simulated draw failure".into());
    let job = mutate(&mut h, &mut py, "stop", json!({}));
    let r = wait_job(&mut h, &job);
    assert_eq!(r["status"], "error", "{r}");
    assert_eq!(r["cleanup"], "unknown");
    assert_eq!(r["native"]["reset"]["outcome"], "failed");
    let s = api_status(&h);
    assert_eq!(
        s["cleanup"], "unknown",
        "closing after a failed reset is unverified"
    );
    assert_eq!(s["phase"], "error");
}

/// The common Python SDK (unmodified 1.0.4) drives this GUI over the real
/// loopback socket. Demo backend only: the script refuses to run unless
/// status reports simulated output.
#[test]
fn python_sdk_drives_the_demo_gui_without_real_output() {
    let run = tempfile::tempdir().unwrap();
    let mut h = harness(true);
    until(&mut h, "display list", |a| !a.displays.is_empty());
    with_gui_api(&mut h, run.path());
    h.state_mut().add_to_experiment(
        "Smoke row".into(),
        slm_core::PatternSpec::Uniform { code: 321 },
    );
    let item = h.state().experiment.experiment.items[0].id.clone();
    let descriptor = h.state().control_api.as_ref().unwrap().descriptor.clone();
    let sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bindings/control");
    let python = std::env::var("PYTHON")
        .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into());
    let mut child = std::process::Command::new(&python)
        .arg("-B")
        .arg(sdk.join("slm_gui_smoke.py"))
        .arg(&descriptor)
        .arg(&item)
        .current_dir(&sdk)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run {python} ({e}); set PYTHON to Python >= 3.10"));
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("Python smoke timed out");
        }
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(5));
    };
    let output = child.wait_with_output().unwrap();
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(status.success(), "Python smoke failed:\n{text}");
    assert!(text.contains("KohdaLab SLM GUI smoke: OK"), "{text}");
    h.run_steps(3);
    let app = h.state();
    assert!(app.control.lease.is_none());
    assert!(matches!(app.device_state, DeviceState::Disconnected));
    let demo = app.demo_control.lock();
    assert!(!demo.open);
    assert_eq!(demo.last_frame.as_ref().unwrap().uniform_code(), Some(0));
    assert!(
        demo.shows >= 6,
        "connect, applies, reset and stop: {}",
        demo.shows
    );
}
