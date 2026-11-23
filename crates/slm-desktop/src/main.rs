#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! KohdaLab SLM.
//!
//! - no arguments: normal GUI with `~/.kohdalab/slm/config/config.json`
//! - `--config <path>`: normal GUI with another config file
//! - `--demo`: simulated output only, temporary config (never opens a display)
//! - `--demo-capture <png> [width height [tab] [expanded] [settings]]`: demo launch check; connects to the simulated
//!   output, steps through the list, saves a screenshot of this window, closes
//! - `--demo-smoke <new-dir>`: headless demo vertical slice; writes evidence
//!   into a new directory and exits non-zero on any failed check
//! - `--gui-api` (with no arguments, `--config <path>`, `--demo` or
//!   `--demo-wavelength`): also start the loopback control adapter (reads,
//!   plus leased connect/apply/reset/disconnect/stop); see
//!   `docs/gui-control-api.md`

mod app;
mod control_api;
mod control_lease;
#[cfg(test)]
mod gui_tests;

#[cfg(test)]
mod icon_tests {
    #[test]
    fn embedded_icon_decodes() {
        let icon = super::app_icon();
        assert!(icon.width >= 16 && icon.width == icon.height);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
mod layout;
mod preview;
mod smoke;
mod theme;

use std::path::PathBuf;

/// Window/taskbar icon (Windows EXE resources come from `build.rs`).
fn app_icon() -> std::sync::Arc<eframe::egui::IconData> {
    std::sync::Arc::new(
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
            .expect("embedded SLM icon"),
    )
}

fn run_gui(options: app::AppOptions) -> Result<(), String> {
    run_gui_sized(options, [1566., 900.], "")
}

fn run_gui_sized(options: app::AppOptions, size: [f32; 2], view: &str) -> Result<(), String> {
    let view = view.to_string();
    let title = if options.demo {
        "KohdaLab SLM · DEMO · SIMULATED"
    } else {
        "KohdaLab SLM"
    };
    eframe::run_native(
        title,
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_icon(app_icon())
                .with_inner_size(size)
                // Small initial minimum so the OS can maximize onto small or
                // scaled screens; the app sets the real minimum (capped by the
                // usable screen) from the first frame.
                .with_min_inner_size([layout::Columns::minimum_width(true, true), 400.]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = app::App::new(&cc.egui_ctx, options);
            app.set_capture_view(&view);
            if view == "wavelength" {
                app.settings_open = true;
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--gui-api` is opt-in and only combines with the interactive launches.
    let gui_api = args.iter().any(|a| a == "--gui-api");
    args.retain(|a| a != "--gui-api");
    let interactive = match args.as_slice() {
        [] => true,
        [flag] => flag == "--demo" || flag == "--demo-wavelength",
        [flag, _] => flag == "--config",
        _ => false,
    };
    let result = match args.as_slice() {
        _ if gui_api && !interactive => {
            Err("--gui-api works only with no args, --config <path>, --demo[-wavelength]".into())
        }
        [] => match slm_device::config::default_config_path() {
            Some(config_path) => run_gui(app::AppOptions {
                config_path,
                demo: false,
                tempdir: None,
                capture: None,
                gui_api,
            }),
            None => Err("Cannot find the home directory for ~/.kohdalab/slm".into()),
        },
        [flag, path] if flag == "--config" => run_gui(app::AppOptions {
            config_path: PathBuf::from(path),
            demo: false,
            tempdir: None,
            capture: None,
            gui_api,
        }),
        [flag] if flag == "--demo" || flag == "--demo-wavelength" => tempfile::tempdir()
            .map_err(|e| format!("Create demo config directory: {e}"))
            .and_then(|dir| {
                run_gui_sized(app::AppOptions {
                    config_path: dir.path().join("config.json"),
                    demo: true,
                    tempdir: Some(dir),
                    capture: None,
                    gui_api,
                }, [1566., 900.], if flag == "--demo-wavelength" { "wavelength" } else { "" })
            }),
        [flag, png, rest @ ..] if flag == "--demo-capture" && rest.len() != 1 => {
            let view = rest.get(2..).map(|v| v.join(" ")).unwrap_or_default();
            let size = match rest {
                [w, h, ..] => match (w.parse::<f32>(), h.parse::<f32>()) {
                    (Ok(w), Ok(h)) => Some([w, h]),
                    _ => None,
                },
                _ => Some([1566., 900.]),
            };
            match size {
                None => Err("--demo-capture <png> [width height]".into()),
                Some(size) => tempfile::tempdir()
                    .map_err(|e| format!("Create demo config directory: {e}"))
                    .and_then(|dir| {
                        run_gui_sized(
                            app::AppOptions {
                                config_path: dir.path().join("config.json"),
                                demo: true,
                                tempdir: Some(dir),
                                capture: Some(PathBuf::from(png)),
                                gui_api: false,
                            },
                            size,
                            &view,
                        )
                    })
                    .and_then(|()| {
                        if std::path::Path::new(png).is_file() {
                            Ok(())
                        } else {
                            Err("Demo capture did not produce a screenshot".into())
                        }
                    }),
            }
        }
        [flag, dir] if flag == "--demo-smoke" => smoke::run(&PathBuf::from(dir)),
        _ => Err("Usage: kohdalab-slm [--gui-api] [--demo | --demo-wavelength | --config <path>] | --demo-capture <png> [w h] | --demo-smoke <new-dir>".into()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
