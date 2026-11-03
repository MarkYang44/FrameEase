//! Input discovery for files, multi-selections, drops and folders.
//!
//! Folders are scanned non-recursively unless asked; output folders and our
//! own temp files are excluded so results are never re-imported.

use crate::fsx;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "asf", "flv", "f4v", "ts", "m2ts", "mts", "m2t", "mpg",
    "mpeg", "mpe", "vob", "3gp", "3g2", "ogv", "mxf", "y4m", "ivf", "obu", "264", "h264", "265", "h265", "hevc",
    "av1", "rm", "rmvb", "dv", "nut",
];

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub recursive: bool,
    /// Output folders to skip (matched by identity).
    pub exclude_dirs: Vec<PathBuf>,
    /// Folder names to skip anywhere (e.g. the default output subfolder).
    pub exclude_dir_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub files: Vec<PathBuf>,
    pub skipped_temp: usize,
    pub skipped_output_dirs: usize,
    pub skipped_non_video: usize,
    pub errors: Vec<String>,
}

pub fn is_video_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Natural order: "ep2" < "ep10", case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Less,
            (Some(_), None) => return Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut ai), take(&mut bi));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                ai.next();
                bi.next();
                let ord = ca.to_lowercase().cmp(cb.to_lowercase());
                if ord != Equal {
                    return ord;
                }
            }
        }
    }
}

pub fn scan_inputs(paths: &[PathBuf], opts: &ScanOptions) -> ScanResult {
    let mut res = ScanResult::default();
    let mut seen = HashSet::new();
    for p in paths {
        match std::fs::metadata(p) {
            Ok(m) if m.is_dir() => scan_dir(p, opts, &mut res, &mut seen),
            Ok(_) => {
                // Explicit picks are honoured whatever the extension (probing decides),
                // except our own temp files.
                if fsx::is_owned_temp(p) {
                    res.skipped_temp += 1;
                } else {
                    push_unique(p, &mut res, &mut seen);
                }
            }
            Err(e) => res.errors.push(format!("{}: {e}", p.display())),
        }
    }
    res
}

fn push_unique(p: &Path, res: &mut ScanResult, seen: &mut HashSet<PathBuf>) {
    let key = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if seen.insert(key) {
        res.files.push(p.to_path_buf());
    }
}

fn is_excluded_dir(dir: &Path, opts: &ScanOptions) -> bool {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    opts.exclude_dir_names.contains(&name) || opts.exclude_dirs.iter().any(|d| fsx::is_same_path(d, dir))
}

/// Files of a folder first (natural order), then its subfolders.
fn scan_dir(dir: &Path, opts: &ScanOptions, res: &mut ScanResult, seen: &mut HashSet<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            res.errors.push(format!("{}: {e}", dir.display()));
            return;
        }
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            dirs.push((name, path));
        } else if ft.is_file() || (ft.is_symlink() && path.is_file()) {
            files.push((name, path));
        }
        // Symlinked directories are not followed (avoids loops).
    }
    files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    dirs.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    for (_, path) in files {
        if fsx::is_owned_temp(&path) {
            res.skipped_temp += 1;
        } else if is_video_ext(&path) {
            push_unique(&path, res, seen);
        } else {
            res.skipped_non_video += 1;
        }
    }
    if opts.recursive {
        for (_, path) in dirs {
            if is_excluded_dir(&path, opts) {
                res.skipped_output_dirs += 1;
            } else {
                scan_dir(&path, opts, res, seen);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    fn names(r: &ScanResult) -> Vec<String> {
        r.files.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        touch(&r.join("第10集.mp4"));
        touch(&r.join("第2集.MKV"));
        touch(&r.join("notes.txt"));
        touch(&r.join("cover.jpg"));
        touch(&r.join(".hidden.mp4"));
        touch(&r.join("第2集_h264.mp4.fe-0123abcd.part"));
        touch(&r.join("子目录").join("旅行 vlog.mov"));
        touch(&r.join("FrameEase 输出").join("第2集_h264.mp4"));
        touch(&r.join("custom-out").join("old_av1.mp4"));
        d
    }

    #[test]
    fn folder_scan_is_non_recursive_by_default() {
        let d = fixture();
        let r = scan_inputs(&[d.path().to_path_buf()], &ScanOptions::default());
        assert_eq!(names(&r), vec!["第2集.MKV", "第10集.mp4"]);
        assert_eq!(r.skipped_temp, 1);
        assert_eq!(r.skipped_non_video, 2);
    }

    #[test]
    fn recursive_scan_excludes_output_folders() {
        let d = fixture();
        let opts = ScanOptions {
            recursive: true,
            exclude_dirs: vec![d.path().join("custom-out")],
            exclude_dir_names: vec!["FrameEase 输出".into()],
        };
        let r = scan_inputs(&[d.path().to_path_buf()], &opts);
        assert_eq!(names(&r), vec!["第2集.MKV", "第10集.mp4", "旅行 vlog.mov"]);
        assert_eq!(r.skipped_output_dirs, 2);
    }

    #[test]
    fn explicit_files_are_kept_even_with_odd_extensions_but_never_temp_files() {
        let d = tempfile::tempdir().unwrap();
        let odd = d.path().join("录像.dat");
        let tmp = d.path().join("x.mp4.fe-0123abcd.part");
        touch(&odd);
        touch(&tmp);
        let r = scan_inputs(&[odd.clone(), odd.clone(), tmp], &ScanOptions::default());
        assert_eq!(r.files, vec![odd], "deduplicated, temp file refused");
        assert_eq!(r.skipped_temp, 1);
    }

    #[test]
    fn missing_paths_are_reported_not_fatal() {
        let r = scan_inputs(&[PathBuf::from("/definitely/not/here.mp4")], &ScanOptions::default());
        assert!(r.files.is_empty());
        assert_eq!(r.errors.len(), 1);
    }

    #[test]
    fn natural_order_sorts_numbers_by_value() {
        let mut v = vec!["ep10.mp4", "Ep2.mp4", "ep1.mp4"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["ep1.mp4", "Ep2.mp4", "ep10.mp4"]);
    }

    #[test]
    fn video_extension_check_is_case_insensitive() {
        assert!(is_video_ext(Path::new("a.MP4")));
        assert!(is_video_ext(Path::new("b.m2ts")));
        assert!(!is_video_ext(Path::new("c.srt")));
        assert!(!is_video_ext(Path::new("noext")));
    }
}
