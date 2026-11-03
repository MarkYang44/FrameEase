//! Locating the FFmpeg/ffprobe backend. Order: user-chosen → bundled with the
//! app → portable folder next to the executable → system locations. GUI apps on
//! macOS do not inherit the shell PATH, so common package-manager folders are
//! searched explicitly; users never have to edit PATH.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolSource {
    Custom,
    Bundled,
    Portable,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub source: ToolSource,
}

pub fn exe_name(base: &str) -> String {
    format!("{base}{}", std::env::consts::EXE_SUFFIX)
}

/// Both binaries in `dir`, if present.
pub fn pair_in(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let ff = dir.join(exe_name("ffmpeg"));
    let fp = dir.join(exe_name("ffprobe"));
    (ff.is_file() && fp.is_file()).then_some((ff, fp))
}

/// `custom` may be a folder or the ffmpeg binary itself.
pub fn locate(custom: Option<&Path>, bundled: Option<&Path>, portable: Option<&Path>) -> Option<FfmpegPaths> {
    let found = |dir: &Path, source| pair_in(dir).map(|(ffmpeg, ffprobe)| FfmpegPaths { ffmpeg, ffprobe, source });
    if let Some(c) = custom {
        let dir = if c.is_file() { c.parent().unwrap_or(c) } else { c };
        if let Some(p) = found(dir, ToolSource::Custom) {
            return Some(p);
        }
    }
    if let Some(p) = bundled.and_then(|b| found(b, ToolSource::Bundled)) {
        return Some(p);
    }
    if let Some(p) = portable.and_then(|b| found(b, ToolSource::Portable)) {
        return Some(p);
    }
    system_dirs().iter().find_map(|d| found(d, ToolSource::System))
}

pub fn system_dirs() -> Vec<PathBuf> {
    #[cfg_attr(windows, allow(unused_mut))]
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    #[cfg(target_os = "macos")]
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
        dirs.push(PathBuf::from(d));
    }
    #[cfg(target_os = "linux")]
    for d in ["/usr/bin", "/usr/local/bin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fake_pair(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(exe_name("ffmpeg")), b"").unwrap();
        fs::write(dir.join(exe_name("ffprobe")), b"").unwrap();
    }

    #[test]
    fn pair_requires_both_binaries() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(exe_name("ffmpeg")), b"").unwrap();
        assert!(pair_in(d.path()).is_none());
        fs::write(d.path().join(exe_name("ffprobe")), b"").unwrap();
        assert!(pair_in(d.path()).is_some());
    }

    #[test]
    fn custom_beats_bundled_and_bundled_beats_portable() {
        let d = tempfile::tempdir().unwrap();
        let (c, b, p) = (d.path().join("custom 目录"), d.path().join("bundled"), d.path().join("portable"));
        fake_pair(&c);
        fake_pair(&b);
        fake_pair(&p);
        assert_eq!(locate(Some(&c), Some(&b), Some(&p)).unwrap().source, ToolSource::Custom);
        assert_eq!(locate(None, Some(&b), Some(&p)).unwrap().source, ToolSource::Bundled);
        let found = locate(None, Some(&d.path().join("missing")), Some(&p)).unwrap();
        assert_eq!(found.source, ToolSource::Portable);
        assert_eq!(found.ffprobe, p.join(exe_name("ffprobe")));
    }

    #[test]
    fn custom_can_point_at_the_ffmpeg_binary() {
        let d = tempfile::tempdir().unwrap();
        fake_pair(d.path());
        let f = locate(Some(&d.path().join(exe_name("ffmpeg"))), None, None).unwrap();
        assert_eq!(f.source, ToolSource::Custom);
        assert_eq!(f.ffmpeg, d.path().join(exe_name("ffmpeg")));
    }

    #[test]
    fn broken_custom_falls_through_to_next_candidate() {
        let d = tempfile::tempdir().unwrap();
        let b = d.path().join("bundled");
        fake_pair(&b);
        let f = locate(Some(&d.path().join("nope")), Some(&b), None).unwrap();
        assert_eq!(f.source, ToolSource::Bundled);
    }
}
