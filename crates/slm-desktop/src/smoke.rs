//! `--demo-smoke <new-dir>`: headless demo vertical slice through the same
//! core and worker paths the GUI uses. Only the demo backend is created.

use slm_core::csv::{read_santec_csv, write_santec_csv};
use slm_core::experiment::{ExperimentDoc, ExperimentItem};
use slm_core::heatmap::HeatmapKind;
use slm_core::sweep::{SweepSpec, SweepTarget};
use slm_core::{
    AngleValue, Geometry, PatternSpec, PeriodUnit, PeriodValue, PhaseValue, WaveComponent, Waveform,
};
use slm_device::demo::{DemoControl, demo_displays};
use slm_device::worker::{DeviceState, default_factory};
use slm_device::{
    ApplyItem, BackendKind, Command, DeviceProfile, DeviceWorker, Event, FrameSource, InputMode,
};
use std::path::Path;
use std::time::{Duration, Instant};

struct Checks {
    lines: Vec<serde_json::Value>,
    failed: usize,
}

impl Checks {
    fn check(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        let detail = detail.into();
        println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            self.failed += 1;
        }
        self.lines
            .push(serde_json::json!({"check": name, "ok": ok, "detail": detail}));
    }
}

fn wait<T>(worker: &DeviceWorker, mut f: impl FnMut(&Event) -> Option<T>) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Some(event) = worker.wait_event(left)
            && let Some(v) = f(&event)
        {
            return Ok(v);
        }
    }
    Err("worker did not answer within 20 s".into())
}

fn wave(period: PeriodValue, waveform: Waveform) -> WaveComponent {
    WaveComponent {
        period,
        waveform,
        initial_phase: PhaseValue::zero(),
    }
}

pub fn run(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        return Err(format!("Smoke output already exists: {}", dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut c = Checks {
        lines: Vec::new(),
        failed: 0,
    };
    let device = DeviceProfile::santec_slm200_candidate();
    let profile = device.generation;
    let geometry = |deg| Geometry {
        center_offset_px: device.default_center_offset_px,
        rotation: AngleValue::degrees(deg),
    };

    // Patterns and sweeps.
    let mut doc = ExperimentDoc::new("demo-smoke");
    doc.push(ExperimentItem::generated(
        "Uniform 1023".into(),
        PatternSpec::Uniform { code: 1023 },
        profile,
    ));
    doc.push(ExperimentItem::generated(
        "Single ramp 200 px 30°".into(),
        PatternSpec::SingleWave {
            wave: wave(PeriodValue::px(200.), Waveform::Ramp),
            geometry: geometry(30.),
        },
        profile,
    ));
    doc.push(ExperimentItem::generated(
        "Single binary q=0.5 µm⁻¹".into(),
        PatternSpec::SingleWave {
            wave: wave(
                PeriodValue {
                    value: 0.5,
                    unit: PeriodUnit::SampleWavevector,
                },
                Waveform::Binary,
            ),
            geometry: geometry(0.),
        },
        profile,
    ));
    doc.push(ExperimentItem::generated(
        "Multi 3 components, 2 px bands, 90°, reversed order".into(),
        PatternSpec::MultiWave {
            components: vec![
                wave(PeriodValue::px(50.), Waveform::Ramp),
                wave(PeriodValue::px(80.), Waveform::Binary),
                wave(
                    PeriodValue {
                        value: 8.,
                        unit: PeriodUnit::SampleUm,
                    },
                    Waveform::Ramp,
                ),
            ],
            geometry: geometry(90.),
            stripe_width_px: Some(2),
            reverse_band_order: true,
        },
        profile,
    ));
    let sweeps = [
        (
            "contrast",
            SweepSpec {
                base: PatternSpec::Uniform { code: 0 },
                target: SweepTarget::UniformCode,
                start: 0.,
                end: 1023.,
                step: 10.,
                include_end: true,
            },
            104usize,
        ),
        (
            "wavevector",
            SweepSpec {
                base: PatternSpec::SingleWave {
                    wave: wave(
                        PeriodValue {
                            value: 0.,
                            unit: PeriodUnit::SampleWavevector,
                        },
                        Waveform::Ramp,
                    ),
                    geometry: geometry(0.),
                },
                target: SweepTarget::Period { component: 0 },
                start: 0.,
                end: 1.,
                step: 0.1,
                include_end: false,
            },
            11,
        ),
        (
            "rotation",
            SweepSpec {
                base: PatternSpec::SingleWave {
                    wave: wave(PeriodValue::px(100.), Waveform::Ramp),
                    geometry: geometry(0.),
                },
                target: SweepTarget::Rotation,
                start: 0.,
                end: 90.,
                step: 10.,
                include_end: false,
            },
            10,
        ),
        (
            "rotation descending",
            SweepSpec {
                base: PatternSpec::SingleWave {
                    wave: wave(PeriodValue::px(100.), Waveform::Ramp),
                    geometry: geometry(0.),
                },
                target: SweepTarget::Rotation,
                start: 90.,
                end: 0.,
                step: 30.,
                include_end: false,
            },
            4,
        ),
    ];
    for (name, spec, expected) in &sweeps {
        let items = spec.expand()?;
        let preview = spec.preview()?;
        c.check(
            &format!("sweep {name}"),
            items.len() == *expected,
            preview.describe(""),
        );
        for (value, s) in items {
            doc.push(ExperimentItem::generated(
                format!("{name} {value}"),
                s,
                profile,
            ));
        }
    }

    // Legacy CSV import (asymmetric pattern so orientation is visible).
    let raster = profile.raster;
    let legacy = slm_core::CodeFrame {
        width: raster.width,
        height: raster.height,
        data: (0..raster.height)
            .flat_map(|y| {
                (0..raster.width).map(move |x| {
                    if x < 200 && y < 100 {
                        1023
                    } else {
                        ((x / 64 + y / 300) % 8 * 100) as u16
                    }
                })
            })
            .collect(),
    };
    let legacy_path = dir.join("legacy_santec.csv");
    std::fs::write(&legacy_path, write_santec_csv(&legacy)).map_err(|e| e.to_string())?;
    doc.import_csv(&legacy_path, profile)?;
    let csv_index = doc.experiment.items.len() - 1;

    // Save, reload, compare.
    let exp_path = dir.join("experiment").join("demo-smoke.json");
    doc.save(&exp_path)?;
    let loaded = ExperimentDoc::load(&exp_path)?;
    c.check(
        "experiment round trip",
        loaded.experiment == doc.experiment,
        format!("{} items", loaded.experiment.items.len()),
    );
    let reloaded_csv = loaded.render_item(csv_index)?;
    c.check(
        "imported CSV preserved",
        reloaded_csv.code() == &legacy,
        reloaded_csv.code().digest(),
    );

    // Export CSV + PNG for the first four items and the CSV.
    let mut subset = loaded.clone();
    subset.experiment.items = vec![
        loaded.experiment.items[0].clone(),
        loaded.experiment.items[1].clone(),
        loaded.experiment.items[3].clone(),
        loaded.experiment.items[csv_index].clone(),
    ];
    let export_dir = dir.join("export");
    let files = subset.export(&export_dir, &HeatmapKind::ALL)?;
    let pngs = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .count();
    c.check(
        "export files",
        pngs == 1 + 3 + 3 + 1,
        format!("{} files, {pngs} PNG", files.len()),
    );
    let csv_item = &subset.experiment.items[3];
    let reexported = std::fs::read_to_string(export_dir.join(format!("004_{}.csv", csv_item.id)))
        .map_err(|e| e.to_string())?;
    let reexported = read_santec_csv(&reexported, raster.width, raster.height, profile.max_code)
        .map_err(|e| e.to_string())?;
    c.check(
        "legacy CSV re-export identical",
        reexported == legacy,
        "codes equal",
    );

    // Demo worker: same command path as the GUI.
    let demo = DemoControl::default();
    let mut worker = DeviceWorker::spawn(default_factory(demo.clone()), Box::new(|| {}));
    let selection = demo_displays()[1].selection();
    worker.send(Command::Connect {
        backend: BackendKind::Demo,
        selection: Some(selection.clone()),
        mode: None,
        profile: device.clone(),
    })?;
    let state = wait(&worker, |e| match e {
        Event::State(DeviceState::Connecting) => None,
        Event::State(s) => Some(s.clone()),
        _ => None,
    })?;
    c.check(
        "unconfirmed mode blocks output",
        state == DeviceState::Disconnected && demo.lock().shows == 0,
        "no frame sent",
    );
    worker.send(Command::Connect {
        backend: BackendKind::Demo,
        selection: Some(selection),
        mode: Some(InputMode::Hz60),
        profile: device.clone(),
    })?;
    let state = wait(&worker, |e| match e {
        Event::State(DeviceState::Connecting) => None,
        Event::State(s) => Some(s.clone()),
        _ => None,
    })?;
    c.check(
        "connect shows reset",
        state.is_connected()
            && demo
                .lock()
                .last_frame
                .as_ref()
                .and_then(|f| f.uniform_code())
                == Some(device.reset_code),
        format!("reset code {}", device.reset_code),
    );
    for (i, item) in loaded
        .experiment
        .items
        .iter()
        .enumerate()
        .filter(|(i, _)| [0usize, 1, 2, 3, csv_index].contains(i))
    {
        let request = worker.request_id();
        worker.send(Command::Apply {
            request,
            item: ApplyItem {
                item_id: Some(item.id.clone()),
                label: item.label.clone(),
                source: FrameSource::Experiment {
                    item: item.clone(),
                    asset: loaded.asset_path(item),
                },
            },
        })?;
        let applied = wait(&worker, |e| match e {
            Event::Applied {
                request: r,
                receipt,
                ..
            } if *r == request => Some(Ok(receipt.digest.clone())),
            Event::ApplyFailed {
                request: r, error, ..
            } if *r == request => Some(Err(error.clone())),
            _ => None,
        })?;
        let expected = loaded.render_item(i)?.code().digest();
        let shown = demo.lock().last_frame.as_ref().map(|f| f.digest());
        c.check(
            &format!("apply item {} ({})", i + 1, item.label),
            applied.as_ref().ok() == Some(&expected) && shown.as_ref() == Some(&expected),
            applied.unwrap_or_else(|e| e),
        );
    }
    // Display renumbering must stop output.
    let mut renumbered = demo_displays();
    renumbered[1].gdi_name = r"\\.\DISPLAY3".into();
    demo.lock().displays = renumbered;
    let request = worker.request_id();
    worker.send(Command::Reset { request })?;
    let refused = wait(&worker, |e| match e {
        Event::ApplyFailed {
            request: r,
            output_known,
            ..
        } if *r == request => Some(!output_known),
        Event::Applied { request: r, .. } if *r == request => Some(false),
        _ => None,
    })?;
    c.check(
        "renumbered display stops output",
        refused && !demo.lock().open,
        "output hidden, state unknown",
    );
    let summary = worker.shutdown(Duration::from_secs(10))?;
    c.check("shutdown", true, summary);

    let report = serde_json::json!({
        "passed": c.failed == 0,
        "failed": c.failed,
        "checks": c.lines,
        "note": "Demo backend only; no display or device was opened.",
    });
    std::fs::write(
        dir.join("smoke-summary.json"),
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if c.failed == 0 {
        println!("Demo smoke passed. Evidence: {}", dir.display());
        Ok(())
    } else {
        Err(format!("{} demo smoke checks failed", c.failed))
    }
}
