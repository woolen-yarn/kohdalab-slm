//! Append-only local session events, separate from the bounded GUI log.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Journal {
    file: File,
    pub path: PathBuf,
    session: String,
}
impl Journal {
    pub fn open(config: &Path) -> Result<Self, String> {
        let parent = config
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let root = if parent.file_name().is_some_and(|n| n == "config") {
            parent.parent().unwrap_or(parent)
        } else {
            parent
        };
        let dir = root.join("logs");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let session = slm_core::experiment::new_id();
        let path = dir.join(format!("slm-session-{session}.jsonl"));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            path,
            session,
        })
    }
    pub fn append(
        &mut self,
        event: &str,
        details: serde_json::Value,
        simulated: bool,
    ) -> Result<(), String> {
        let row = serde_json::json!({
            "schema_version": 1, "application": "kohdalab-slm",
            "application_version": env!("CARGO_PKG_VERSION"), "session_id": self.session,
            "utc_unix_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()),
            "event": event, "simulated": simulated, "details": details,
        });
        serde_json::to_writer(&mut self.file, &row).map_err(|e| e.to_string())?;
        self.file
            .write_all(b"\n")
            .and_then(|_| self.file.flush())
            .and_then(|_| self.file.sync_data())
            .map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_are_unique_and_events_are_durable_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config/config.json");
        let mut journal = Journal::open(&config).unwrap();
        let other = Journal::open(&config).unwrap();
        assert_ne!(journal.path, other.path);
        journal
            .append(
                "applied",
                serde_json::json!({"item_id":"one", "frame_sha256":"abc"}),
                true,
            )
            .unwrap();
        journal
            .append("reset", serde_json::json!({"success":false}), true)
            .unwrap();
        let text = std::fs::read_to_string(&journal.path).unwrap();
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["details"]["item_id"], "one");
        assert_eq!(rows[0]["simulated"], true);
        assert!(rows[0]["utc_unix_ms"].as_u64().unwrap() > 0);
        assert!(journal.path.starts_with(dir.path().join("logs")));
    }
}
