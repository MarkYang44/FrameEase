//! Results manifest (CSV) that tolerates being locked by Excel or other apps.
//!
//! Rows are buffered; a write failure (sharing violation, read-only file,
//! missing folder…) only defers them. Nothing here can fail or block a job.

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const HEADER: [&str; 13] = [
    "时间", "状态", "源文件", "输出文件", "处理方式", "视频编码器", "参数", "源大小(字节)", "输出大小(字节)",
    "时长(秒)", "耗时(秒)", "校验", "错误",
];

#[derive(Debug, Clone, Default)]
pub struct ManifestRow {
    pub time: String,
    pub status: String,
    pub source: String,
    pub output: String,
    pub mode: String,
    pub encoder: String,
    pub params: String,
    pub source_size: Option<u64>,
    pub output_size: Option<u64>,
    pub duration_s: Option<f64>,
    pub elapsed_s: f64,
    pub verify: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestStatus {
    pub path: PathBuf,
    /// Rows waiting to be written because the file could not be opened.
    pub pending: usize,
    pub last_error: Option<String>,
}

pub struct Manifest {
    path: PathBuf,
    inner: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    rows: Vec<String>,
    last_error: Option<String>,
}

impl Manifest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), inner: Mutex::new(Pending::default()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Queue a row and try to write everything pending. Never fails.
    pub fn record(&self, row: &ManifestRow) -> ManifestStatus {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.rows.push(format_row(row));
        self.write_pending(&mut g);
        self.status_of(&g)
    }

    /// Retry writing pending rows. Never fails.
    pub fn flush(&self) -> ManifestStatus {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !g.rows.is_empty() {
            self.write_pending(&mut g);
        }
        self.status_of(&g)
    }

    pub fn status(&self) -> ManifestStatus {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.status_of(&g)
    }

    fn status_of(&self, g: &Pending) -> ManifestStatus {
        ManifestStatus { path: self.path.clone(), pending: g.rows.len(), last_error: g.last_error.clone() }
    }

    fn write_pending(&self, g: &mut Pending) {
        match self.try_write(&g.rows) {
            Ok(()) => {
                g.rows.clear();
                g.last_error = None;
            }
            Err(e) => g.last_error = Some(e.to_string()),
        }
    }

    /// One open + one write per attempt; rows stay queued if anything fails.
    fn try_write(&self, rows: &[String]) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        let mut buf = String::new();
        if f.metadata()?.len() == 0 {
            buf.push('\u{feff}');
            buf.push_str(&HEADER.iter().map(|h| csv_field(h)).collect::<Vec<_>>().join(","));
            buf.push_str("\r\n");
        }
        for r in rows {
            buf.push_str(r);
            buf.push_str("\r\n");
        }
        f.write_all(buf.as_bytes())?;
        f.flush()
    }
}

fn format_row(r: &ManifestRow) -> String {
    let opt_u = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_default();
    let fields = [
        r.time.clone(),
        r.status.clone(),
        r.source.clone(),
        r.output.clone(),
        r.mode.clone(),
        r.encoder.clone(),
        r.params.clone(),
        opt_u(r.source_size),
        opt_u(r.output_size),
        r.duration_s.map(|d| format!("{d:.3}")).unwrap_or_default(),
        format!("{:.1}", r.elapsed_s),
        r.verify.clone(),
        r.error.clone(),
    ];
    fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",")
}

/// RFC 4180 field quoting.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn row(src: &str) -> ManifestRow {
        ManifestRow {
            time: "2026-11-03 18:00:00".into(),
            status: "完成".into(),
            source: src.into(),
            output: "/out/a_av1.mp4".into(),
            mode: "有损".into(),
            encoder: "libsvtav1".into(),
            params: "crf=35 preset=7".into(),
            source_size: Some(1000),
            output_size: Some(400),
            duration_s: Some(12.5),
            elapsed_s: 3.25,
            verify: "通过".into(),
            error: String::new(),
        }
    }

    fn data_lines(path: &Path) -> Vec<String> {
        let text = fs::read_to_string(path).unwrap();
        text.trim_start_matches('\u{feff}').lines().skip(1).map(String::from).collect()
    }

    #[test]
    fn writes_bom_and_header_once_then_appends() {
        let d = tempfile::tempdir().unwrap();
        let m = Manifest::new(d.path().join("结果.csv"));
        m.record(&row("/v/一.mp4"));
        let st = m.record(&row("/v/二.mp4"));
        assert_eq!(st.pending, 0);
        let text = fs::read_to_string(m.path()).unwrap();
        assert!(text.starts_with('\u{feff}'), "UTF-8 BOM so Excel shows Chinese correctly");
        assert_eq!(text.matches("源文件").count(), 1, "header written once");
        let rows = data_lines(m.path());
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("/v/一.mp4") && rows[1].contains("/v/二.mp4"));
    }

    #[test]
    fn quotes_fields_with_separators() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    /// Regression: a results CSV held open by Excel used to abort the whole queue.
    /// A locked manifest must only defer rows, then catch up once released.
    #[test]
    fn locked_manifest_defers_rows_and_catches_up_later() {
        let d = tempfile::tempdir().unwrap();
        let m = Manifest::new(d.path().join("results.csv"));
        m.record(&row("/v/before-lock.mp4"));

        let lock = lock_file(m.path());
        let st = m.record(&row("/v/during-lock-1.mp4"));
        assert_eq!(st.pending, 1, "row kept in memory while locked");
        assert!(st.last_error.is_some());
        let st = m.record(&row("/v/during-lock-2.mp4"));
        assert_eq!(st.pending, 2);
        assert_eq!(m.flush().pending, 2, "still locked: flush defers again without error");
        unlock_file(m.path(), lock);

        let st = m.flush();
        assert_eq!(st.pending, 0);
        assert!(st.last_error.is_none());
        let rows = data_lines(m.path());
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("during-lock-1") && rows[2].contains("during-lock-2"), "order preserved");
    }

    #[test]
    fn missing_folder_is_deferred_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let m = Manifest::new(d.path().join("gone").join("results.csv"));
        let st = m.record(&row("/v/x.mp4"));
        assert_eq!(st.pending, 1);
        fs::create_dir(d.path().join("gone")).unwrap();
        assert_eq!(m.flush().pending, 0);
    }

    // Windows: hold the file open without sharing, exactly like Excel does.
    #[cfg(windows)]
    fn lock_file(p: &Path) -> Option<fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        Some(fs::OpenOptions::new().read(true).write(true).share_mode(0).open(p).unwrap())
    }
    #[cfg(windows)]
    fn unlock_file(_p: &Path, lock: Option<fs::File>) {
        drop(lock);
    }

    // macOS/Linux have no mandatory locks; a read-only file reproduces the same open failure.
    #[cfg(unix)]
    fn lock_file(p: &Path) -> Option<fs::File> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o444)).unwrap();
        None
    }
    #[cfg(unix)]
    fn unlock_file(p: &Path, _lock: Option<fs::File>) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o644)).unwrap();
    }
}
