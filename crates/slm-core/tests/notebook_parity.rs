//! Pixel parity with the SLM.ipynb formulas (see tests/reference/notebook_reference.py).

use slm_core::{
    AngleValue, GenerationProfile, Geometry, Optics, PatternSpec, PeriodValue, PhaseCalibration,
    PhaseValue, Raster, WaveComponent, Waveform,
};

fn fixtures() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/reference/notebook_fixtures.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture file"))
        .expect("fixture JSON")
}

fn profile(width: u32, height: u32) -> GenerationProfile {
    GenerationProfile {
        raster: Raster { width, height },
        max_code: 1023,
        optics: Optics {
            pixel_pitch_um: 8.,
            magnification: 1. / 75.,
        },
        calibration: PhaseCalibration {
            phase0: 136.,
            code_period: 888,
        },
        mount_angle_deg: 0.,
        default_stripe_width_px: 1,
    }
}

fn spec(case: &serde_json::Value) -> PatternSpec {
    let periods: Vec<f64> = case["periods_px"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let offset = case["center_offset_px"].as_array().unwrap();
    let geometry = Geometry {
        center_offset_px: [
            offset[0].as_i64().unwrap() as i32,
            offset[1].as_i64().unwrap() as i32,
        ],
        rotation: AngleValue::degrees(case["rotation_deg"].as_f64().unwrap()),
    };
    let wave = |p: f64, waveform| WaveComponent {
        period: PeriodValue::px(p),
        waveform,
        initial_phase: PhaseValue::zero(),
    };
    match case["pattern_type"].as_str().unwrap() {
        "single_wave" => PatternSpec::SingleWave {
            wave: wave(periods[0], Waveform::Ramp),
            geometry,
        },
        "binary_wave" => PatternSpec::SingleWave {
            wave: wave(periods[0], Waveform::Binary),
            geometry,
        },
        "multi_wave" | "double_wave" => PatternSpec::MultiWave {
            components: periods.iter().map(|&p| wave(p, Waveform::Ramp)).collect(),
            geometry,
            stripe_width_px: Some(case["band_width"].as_u64().unwrap() as u32),
            reverse_band_order: case["reverse_band_order"].as_bool().unwrap_or(false),
        },
        other => panic!("unknown pattern type {other}"),
    }
}

#[test]
fn small_rasters_match_notebook() {
    let fixtures = fixtures();
    let mut checked = 0;
    for case in fixtures["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let width = case["width"].as_u64().unwrap() as u32;
        let height = case["height"].as_u64().unwrap() as u32;
        let expected: Vec<u16> = case["codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u16)
            .collect();
        let rendered = spec(case).render(&profile(width, height)).unwrap();
        let actual = &rendered.code().data;
        let mismatches = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
        if case["expect_match"].as_bool().unwrap() {
            assert_eq!(
                mismatches, 0,
                "{name}: {mismatches} pixels differ from the notebook"
            );
        } else {
            // Default band order at 90° (proper rotation, −X) differs from
            // the notebook; Reverse Band Order is the notebook-compatible
            // choice. Keep the difference visible.
            assert!(mismatches > 0, "{name}: expected the documented deviation");
        }
        checked += 1;
    }
    assert!(checked >= 16);
    // Required success condition: 90°, 3 components, 2 px bands, reversed.
    let required = fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "double_3comp_w2_90")
        .expect("required 90° 3-component 2 px case");
    assert!(required["expect_match"].as_bool().unwrap());
    assert!(required["reverse_band_order"].as_bool().unwrap());
}

#[test]
fn full_santec_raster_matches_notebook_hash() {
    let fixtures = fixtures();
    let full = &fixtures["full_raster"];
    let rendered = spec(full).render(&profile(1920, 1200)).unwrap();
    assert_eq!(
        rendered.code().digest(),
        full["code_sha256"].as_str().unwrap()
    );
}
