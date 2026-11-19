//! Versioned experiment files: an ordered list of generated conditions and
//! imported Santec CSVs.
//!
//! `name.json` stores the list. Imported CSVs are copied to the sibling
//! directory `name.assets/` and referenced by relative path and SHA-256. Every
//! item keeps the generation profile (raster, optics, calibration, mounting
//! angle) that was active when it was added, so regeneration does not depend on
//! the PC's current profile.

use crate::csv::{read_santec_csv, write_santec_csv};
use crate::frame::{Rendered, sha256_hex};
use crate::heatmap::{HeatmapKind, write_heatmap};
use crate::pattern::{GenerationProfile, PatternSpec};
use crate::sweep::SweepTarget;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const EXPERIMENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Experiment {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub created_unix_s: u64,
    pub items: Vec<ExperimentItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentItem {
    /// Stable identifier used by exports and (later) the Python API.
    pub id: String,
    pub label: String,
    pub profile: GenerationProfile,
    pub source: ItemSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep: Option<SweepOrigin>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemSource {
    Generated {
        spec: PatternSpec,
    },
    /// Pixel codes are sent exactly as stored: no rotation or calibration.
    ImportedCsv {
        /// Relative to the experiment file's directory.
        file: String,
        sha256: String,
        original_name: String,
        width: u32,
        height: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SweepOrigin {
    pub sweep_id: String,
    pub index: usize,
    pub count: usize,
    pub target: SweepTarget,
    pub target_label: String,
    pub value: f64,
    pub unit: String,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_unix_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl ExperimentItem {
    pub fn generated(label: String, spec: PatternSpec, profile: GenerationProfile) -> Self {
        Self {
            id: new_id(),
            label,
            profile,
            source: ItemSource::Generated { spec },
            sweep: None,
        }
    }
    pub fn kind_label(&self) -> &'static str {
        match &self.source {
            ItemSource::Generated { spec } => spec.kind_label(),
            ItemSource::ImportedCsv { .. } => "Santec CSV",
        }
    }
    pub fn summary(&self) -> String {
        match &self.source {
            ItemSource::Generated { spec } => spec.summary(&self.profile.optics),
            ItemSource::ImportedCsv {
                original_name,
                sha256,
                ..
            } => format!(
                "CSV {original_name} (sha256 {}…)",
                sha256.get(..12).unwrap_or(sha256)
            ),
        }
    }
}

/// An experiment with its location and imported files not yet copied.
#[derive(Clone, Debug)]
pub struct ExperimentDoc {
    pub experiment: Experiment,
    pub path: Option<PathBuf>,
    /// Item ID → absolute source path for CSVs imported since the last save.
    pub pending_assets: HashMap<String, PathBuf>,
    pub dirty: bool,
}

impl Default for ExperimentDoc {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

pub fn assets_dir_name(experiment_path: &Path) -> String {
    let stem = experiment_path
        .file_stem()
        .map_or_else(|| "experiment".into(), |s| s.to_string_lossy().into_owned());
    format!("{stem}.assets")
}

impl ExperimentDoc {
    pub fn new(name: &str) -> Self {
        Self {
            experiment: Experiment {
                schema_version: EXPERIMENT_SCHEMA_VERSION,
                id: new_id(),
                name: name.into(),
                created_unix_s: now_unix_s(),
                items: Vec::new(),
            },
            path: None,
            pending_assets: HashMap::new(),
            dirty: false,
        }
    }

    pub fn push(&mut self, item: ExperimentItem) {
        self.experiment.items.push(item);
        self.dirty = true;
    }

    /// Validate a Santec CSV against `profile` and add it (copied on save).
    pub fn import_csv(
        &mut self,
        source: &Path,
        profile: GenerationProfile,
    ) -> Result<String, String> {
        let bytes = std::fs::read(source).map_err(|e| format!("Read {}: {e}", source.display()))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| format!("{} is not UTF-8 text", source.display()))?;
        read_santec_csv(
            text,
            profile.raster.width,
            profile.raster.height,
            profile.max_code,
        )
        .map_err(|e| format!("{}: {e}", source.display()))?;
        let original_name = source.file_name().map_or_else(
            || "pattern.csv".into(),
            |n| n.to_string_lossy().into_owned(),
        );
        let id = new_id();
        self.pending_assets.insert(
            id.clone(),
            std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf()),
        );
        self.push(ExperimentItem {
            id: id.clone(),
            label: original_name.clone(),
            profile,
            source: ItemSource::ImportedCsv {
                file: String::new(),
                sha256: sha256_hex(&bytes),
                original_name,
                width: profile.raster.width,
                height: profile.raster.height,
            },
            sweep: None,
        });
        Ok(id)
    }

    /// Absolute location of an imported CSV's current bytes.
    pub fn asset_path(&self, item: &ExperimentItem) -> Option<PathBuf> {
        self.resolve_asset(item).ok()
    }

    /// Locate an imported CSV inside the experiment's directory.
    ///
    /// The stored reference is `<name>.assets/<item-id>.csv`. If that is
    /// missing (the experiment file was renamed together with its assets
    /// directory), `<current stem>.assets/<item-id>.csv` is used. The result
    /// must stay inside the experiment directory after resolving symlinks.
    pub fn resolve_asset(&self, item: &ExperimentItem) -> Result<PathBuf, String> {
        if let Some(path) = self.pending_assets.get(&item.id) {
            return Ok(path.clone());
        }
        let ItemSource::ImportedCsv { file, .. } = &item.source else {
            return Err("Item has no imported file".into());
        };
        validate_asset_ref(file, &item.id)?;
        let experiment = self
            .path
            .as_ref()
            .ok_or("Experiment has not been saved, so its assets have no location")?;
        let base = experiment.parent().unwrap_or_else(|| Path::new("."));
        let stored = base.join(file);
        let renamed = base
            .join(assets_dir_name(experiment))
            .join(format!("{}.csv", item.id));
        let candidate = if stored.is_file() || !renamed.is_file() {
            stored
        } else {
            renamed
        };
        if let (Ok(root), Ok(real)) = (
            std::fs::canonicalize(base),
            std::fs::canonicalize(&candidate),
        ) && !real.starts_with(&root)
        {
            return Err(format!(
                "{} resolves outside the experiment directory",
                candidate.display()
            ));
        }
        Ok(candidate)
    }

    pub fn render_item(&self, index: usize) -> Result<Rendered, String> {
        let item = self
            .experiment
            .items
            .get(index)
            .ok_or("Experiment item does not exist")?;
        match &item.source {
            ItemSource::ImportedCsv { .. } => {
                let asset = self.resolve_asset(item)?;
                render_item(item, Some(&asset))
            }
            ItemSource::Generated { .. } => render_item(item, None),
        }
    }

    /// Save to `path`, copying imported CSVs into `<stem>.assets/`.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let assets_name = assets_dir_name(path);
        let assets_dir = base.join(&assets_name);
        let mut updated = self.experiment.clone();
        for (original, item) in self.experiment.items.iter().zip(&mut updated.items) {
            let ItemSource::ImportedCsv { file, sha256, .. } = &mut item.source else {
                continue;
            };
            let current = self
                .asset_path(original)
                .ok_or_else(|| format!("Imported CSV for item {} has no source file", item.id))?;
            let relative = format!("{assets_name}/{}.csv", item.id);
            let target = base.join(&relative);
            let same = std::fs::canonicalize(&current).ok() == std::fs::canonicalize(&target).ok()
                && target.exists();
            if !same {
                let bytes = std::fs::read(&current)
                    .map_err(|e| format!("Read imported CSV {}: {e}", current.display()))?;
                if sha256_hex(&bytes) != *sha256 {
                    return Err(format!(
                        "Imported CSV {} changed since it was added (checksum mismatch)",
                        current.display()
                    ));
                }
                std::fs::create_dir_all(&assets_dir)
                    .map_err(|e| format!("Create {}: {e}", assets_dir.display()))?;
                std::fs::write(&target, &bytes)
                    .map_err(|e| format!("Write {}: {e}", target.display()))?;
            }
            *file = relative;
        }
        let json = serde_json::to_string_pretty(&updated).map_err(|e| e.to_string())?;
        write_atomic(path, json.as_bytes())?;
        self.experiment = updated;
        self.path = Some(path.to_path_buf());
        self.pending_assets.clear();
        self.dirty = false;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("Read {}: {e}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("{}: invalid JSON: {e}", path.display()))?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("{}: missing schema_version", path.display()))?;
        if version != u64::from(EXPERIMENT_SCHEMA_VERSION) {
            return Err(format!(
                "{}: experiment schema {version} is not supported (this build reads {EXPERIMENT_SCHEMA_VERSION})",
                path.display()
            ));
        }
        let experiment: Experiment =
            serde_json::from_value(value).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut ids = std::collections::HashSet::new();
        for (index, item) in experiment.items.iter().enumerate() {
            let at = format!("{}: item {}", path.display(), index + 1);
            uuid::Uuid::parse_str(&item.id)
                .map_err(|_| format!("{at}: ID {:?} is not a UUID", item.id))?;
            if !ids.insert(item.id.as_str()) {
                return Err(format!("{at}: duplicate item ID {}", item.id));
            }
            item.profile.validate().map_err(|e| format!("{at}: {e}"))?;
            if let ItemSource::ImportedCsv {
                file,
                sha256,
                width,
                height,
                ..
            } = &item.source
            {
                if !is_sha256_hex(sha256) {
                    return Err(format!(
                        "{at}: sha256 must be 64 lowercase hexadecimal characters"
                    ));
                }
                validate_asset_ref(file, &item.id).map_err(|e| format!("{at}: {e}"))?;
                if (*width, *height) != (item.profile.raster.width, item.profile.raster.height) {
                    return Err(format!(
                        "{at}: CSV size {width}×{height} does not match the item profile raster"
                    ));
                }
            }
        }
        Ok(Self {
            experiment,
            path: Some(path.to_path_buf()),
            pending_assets: HashMap::new(),
            dirty: false,
        })
    }

    /// Write a Santec CSV for each item and the selected heatmaps, plus a
    /// `manifest.json` mapping files to item IDs. Returns written paths.
    pub fn export(&self, dir: &Path, heatmaps: &[HeatmapKind]) -> Result<Vec<PathBuf>, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("Create {}: {e}", dir.display()))?;
        let mut written = Vec::new();
        let mut manifest = Vec::new();
        for (index, item) in self.experiment.items.iter().enumerate() {
            let rendered = self
                .render_item(index)
                .map_err(|e| format!("Item {} ({}): {e}", index + 1, item.label))?;
            let stem = format!("{:03}_{}", index + 1, item.id);
            let csv = dir.join(format!("{stem}.csv"));
            write_atomic(&csv, write_santec_csv(rendered.code()).as_bytes())?;
            written.push(csv.clone());
            let mut images = Vec::new();
            for &kind in heatmaps {
                if kind != HeatmapKind::Code && rendered.phase().is_none() {
                    continue;
                }
                let png = dir.join(format!("{stem}_{}.png", kind.suffix()));
                write_heatmap(&rendered, kind, item.profile.max_code, &png)?;
                images.push(png.file_name().map(|n| n.to_string_lossy().into_owned()));
                written.push(png);
            }
            manifest.push(serde_json::json!({
                "index": index + 1,
                "item_id": item.id,
                "label": item.label,
                "summary": item.summary(),
                "csv": csv.file_name().map(|n| n.to_string_lossy().into_owned()),
                "code_sha256": rendered.code().digest(),
                "heatmaps": images,
            }));
        }
        let manifest_path = dir.join("manifest.json");
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "experiment_id": self.experiment.id,
            "experiment_name": self.experiment.name,
            "items": manifest,
        }))
        .map_err(|e| e.to_string())?;
        write_atomic(&manifest_path, json.as_bytes())?;
        written.push(manifest_path);
        Ok(written)
    }
}

/// 64 lowercase ASCII hex characters, as written by [`sha256_hex`].
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// An imported-asset reference must be exactly `<dir>.assets/<item-id>.csv`:
/// relative, two plain components, no `..`, separators or drive prefixes.
pub fn validate_asset_ref(file: &str, item_id: &str) -> Result<(), String> {
    let invalid = || {
        format!(
            "Asset path {file:?} must be <experiment>.assets/{item_id}.csv inside the experiment directory"
        )
    };
    if file.contains('\\') || file.contains(':') || file.starts_with('/') {
        return Err(invalid());
    }
    let parts: Vec<&str> = file.split('/').collect();
    let [dir, name] = parts.as_slice() else {
        return Err(invalid());
    };
    let stem = dir.strip_suffix(".assets").unwrap_or("");
    if stem.is_empty()
        || stem.starts_with('.')
        || *name != format!("{item_id}.csv")
        || matches!(*dir, "." | "..")
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn render_item(item: &ExperimentItem, asset: Option<&Path>) -> Result<Rendered, String> {
    match &item.source {
        ItemSource::Generated { spec } => spec.render(&item.profile),
        ItemSource::ImportedCsv {
            sha256,
            width,
            height,
            original_name,
            ..
        } => {
            let path =
                asset.ok_or_else(|| format!("Imported CSV {original_name} has no stored file"))?;
            let bytes = std::fs::read(path).map_err(|e| format!("Read {}: {e}", path.display()))?;
            if sha256_hex(&bytes) != *sha256 {
                return Err(format!(
                    "{} does not match its recorded checksum",
                    path.display()
                ));
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| format!("{} is not UTF-8", path.display()))?;
            read_santec_csv(text, *width, *height, item.profile.max_code)
                .map(Rendered::Code)
                .map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

/// Write via a temporary sibling and rename, so a crash never leaves a
/// half-written experiment or config.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("Create {}: {e}", dir.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("Write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Replace {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{PhaseCalibration, Raster};
    use crate::sweep::SweepSpec;
    use crate::units::Optics;

    fn profile() -> GenerationProfile {
        GenerationProfile {
            raster: Raster {
                width: 6,
                height: 4,
            },
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

    #[test]
    fn save_reload_import_and_export() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = ExperimentDoc::new("demo");
        let sweep = SweepSpec {
            base: PatternSpec::Uniform { code: 0 },
            target: SweepTarget::UniformCode,
            start: 0.,
            end: 20.,
            step: 10.,
            include_end: false,
        };
        for (value, spec) in sweep.expand().unwrap() {
            doc.push(ExperimentItem::generated(
                format!("code {value}"),
                spec,
                profile(),
            ));
        }
        let legacy = dir.path().join("legacy.csv");
        let frame = crate::frame::CodeFrame {
            width: 6,
            height: 4,
            data: (0..24).collect(),
        };
        std::fs::write(&legacy, write_santec_csv(&frame)).unwrap();
        doc.import_csv(&legacy, profile()).unwrap();
        let bad = dir.path().join("bad.csv");
        std::fs::write(&bad, "Y/X,0\n0,1\n").unwrap();
        assert!(doc.import_csv(&bad, profile()).is_err());

        let path = dir.path().join("exp").join("run.json");
        doc.save(&path).unwrap();
        // The legacy file can now disappear; the experiment owns a copy.
        std::fs::remove_file(&legacy).unwrap();
        let loaded = ExperimentDoc::load(&path).unwrap();
        assert_eq!(loaded.experiment, doc.experiment);
        assert_eq!(loaded.experiment.items.len(), 4);
        let csv_item = loaded.render_item(3).unwrap();
        assert_eq!(csv_item.code(), &frame);

        // Save As to another directory copies the asset.
        let mut moved = loaded.clone();
        let other = dir.path().join("other").join("copy.json");
        moved.save(&other).unwrap();
        assert!(other.parent().unwrap().join("copy.assets").is_dir());
        assert_eq!(
            ExperimentDoc::load(&other)
                .unwrap()
                .render_item(3)
                .unwrap()
                .code(),
            &frame
        );

        let out = dir.path().join("export");
        let files = loaded.export(&out, &HeatmapKind::ALL).unwrap();
        assert!(files.iter().any(|p| p.ends_with("manifest.json")));
        let id = &loaded.experiment.items[3].id;
        let exported = std::fs::read_to_string(out.join(format!("004_{id}.csv"))).unwrap();
        assert_eq!(read_santec_csv(&exported, 6, 4, 1023).unwrap(), frame);
    }

    fn saved_with_csv(dir: &Path) -> (PathBuf, String) {
        let mut doc = ExperimentDoc::new("x");
        let legacy = dir.join("a.csv");
        std::fs::write(
            &legacy,
            write_santec_csv(&crate::frame::CodeFrame::uniform(6, 4, 3)),
        )
        .unwrap();
        doc.import_csv(&legacy, profile()).unwrap();
        let path = dir.join("exp").join("x.json");
        doc.save(&path).unwrap();
        (path, doc.experiment.items[0].id.clone())
    }

    fn edit(path: &Path, from: &str, to: &str) {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "{from} not in file");
        std::fs::write(path, text.replacen(from, to, 1)).unwrap();
    }

    #[test]
    fn load_validates_ids_hashes_and_asset_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (path, id) = saved_with_csv(dir.path());
        let good = std::fs::read_to_string(&path).unwrap();
        let reset = || std::fs::write(&path, &good).unwrap();

        edit(&path, &format!("\"id\": \"{id}\""), "\"id\": \"item-1\"");
        assert!(
            ExperimentDoc::load(&path)
                .unwrap_err()
                .contains("not a UUID")
        );
        reset();
        let doc = ExperimentDoc::load(&path).unwrap();
        let ItemSource::ImportedCsv { sha256, file, .. } = &doc.experiment.items[0].source else {
            panic!("csv item expected")
        };
        let (sha256, file) = (sha256.clone(), file.clone());
        // Multibyte / short / uppercase hashes are refused without panicking.
        for bad in ["é".repeat(32), "abc".into(), sha256.to_uppercase()] {
            reset();
            edit(&path, &sha256, &bad);
            assert!(ExperimentDoc::load(&path).unwrap_err().contains("sha256"));
        }
        for bad in [
            format!("../{id}.csv"),
            format!("x.assets/../../{id}.csv"),
            format!("/tmp/{id}.csv"),
            format!("C:x.assets/{id}.csv"),
            format!("x.assets\\\\{id}.csv"),
            "x.assets/other.csv".to_string(),
            format!("x/{id}.csv"),
            format!("..assets/{id}.csv"),
        ] {
            reset();
            edit(
                &path,
                &format!("\"file\": \"{file}\""),
                &format!("\"file\": {}", serde_json::to_string(&bad).unwrap()),
            );
            assert!(ExperimentDoc::load(&path).is_err(), "accepted {bad}");
        }
        reset();
        // summary never slices inside a character.
        let mut item = doc.experiment.items[0].clone();
        if let ItemSource::ImportedCsv { sha256, .. } = &mut item.source {
            *sha256 = "é".repeat(10);
        }
        assert!(item.summary().contains("CSV"));
    }

    #[test]
    fn moved_or_renamed_experiment_keeps_assets() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = saved_with_csv(dir.path());
        // Move the whole directory.
        let moved = dir.path().join("moved");
        std::fs::rename(path.parent().unwrap(), &moved).unwrap();
        let json = moved.join("x.json");
        assert!(ExperimentDoc::load(&json).unwrap().render_item(0).is_ok());
        // Rename only the JSON: the stored x.assets reference still works.
        let renamed = moved.join("y.json");
        std::fs::rename(&json, &renamed).unwrap();
        assert!(
            ExperimentDoc::load(&renamed)
                .unwrap()
                .render_item(0)
                .is_ok()
        );
        // Rename the assets directory to match: the fallback finds it.
        std::fs::rename(moved.join("x.assets"), moved.join("y.assets")).unwrap();
        assert!(
            ExperimentDoc::load(&renamed)
                .unwrap()
                .render_item(0)
                .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn asset_symlink_outside_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (path, id) = saved_with_csv(dir.path());
        let assets = path.parent().unwrap().join("x.assets");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::rename(
            assets.join(format!("{id}.csv")),
            outside.join(format!("{id}.csv")),
        )
        .unwrap();
        std::fs::remove_dir(&assets).unwrap();
        std::os::unix::fs::symlink(&outside, &assets).unwrap();
        let err = ExperimentDoc::load(&path)
            .unwrap()
            .render_item(0)
            .unwrap_err();
        assert!(
            err.contains("no stored file") || err.contains("outside"),
            "{err}"
        );
    }

    #[test]
    fn rejects_tampered_asset_and_unknown_schema() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = ExperimentDoc::new("x");
        let legacy = dir.path().join("a.csv");
        std::fs::write(
            &legacy,
            write_santec_csv(&crate::frame::CodeFrame::uniform(6, 4, 3)),
        )
        .unwrap();
        doc.import_csv(&legacy, profile()).unwrap();
        let path = dir.path().join("x.json");
        doc.save(&path).unwrap();
        let asset = dir
            .path()
            .join("x.assets")
            .join(format!("{}.csv", doc.experiment.items[0].id));
        std::fs::write(
            &asset,
            write_santec_csv(&crate::frame::CodeFrame::uniform(6, 4, 4)),
        )
        .unwrap();
        assert!(ExperimentDoc::load(&path).unwrap().render_item(0).is_err());
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\"schema_version\": 1", "\"schema_version\": 99");
        std::fs::write(&path, text).unwrap();
        assert!(ExperimentDoc::load(&path).is_err());
    }
}
