//! File safety: output naming, same-path protection, owned temp files,
//! atomic no-overwrite commit and free-space checks.
//!
//! Invariants:
//! - FFmpeg only ever writes to a unique temp file owned by the job.
//! - A temp file becomes the final output through an atomic rename that fails
//!   if the destination exists, so neither the source nor an existing output
//!   can be overwritten, even under races.
//! - Deletion is restricted to files matching the owned-temp pattern.

use crate::settings::{ConflictPolicy, OutputDir, OutputSettings};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

const TEMP_MARK: &str = ".fe-";
const TEMP_EXT: &str = ".part";
const MAX_NAME_BYTES: usize = 200;

/// Make a string safe as a file name on Windows, macOS and Linux while keeping
/// Unicode (Chinese, Japanese, emoji), spaces and harmless symbols intact.
pub fn sanitize_file_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 || c == '\u{7f}' => '_',
            c => c,
        })
        .collect();
    trim_trailing_dots_spaces(&mut s);
    if s.len() > MAX_NAME_BYTES {
        let mut cut = MAX_NAME_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        trim_trailing_dots_spaces(&mut s);
    }
    if s.is_empty() {
        return "output".to_string();
    }
    let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    if device {
        s.insert(0, '_');
    }
    s
}

fn trim_trailing_dots_spaces(s: &mut String) {
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
}

/// Expand `{name}`, `{codec}`, `{mode}`, `{date}` and sanitize the result.
/// Single pass, so a `{codec}` inside the file name itself is left alone.
pub fn render_name(template: &str, stem: &str, codec: &str, mode: &str, date: &str) -> String {
    let mut out = String::with_capacity(template.len() + stem.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let token = after.find('}').map(|close| &after[..=close]);
        let value = match token {
            Some("{name}") => Some(stem),
            Some("{codec}") => Some(codec),
            Some("{mode}") => Some(mode),
            Some("{date}") => Some(date),
            _ => None,
        };
        match (token, value) {
            (Some(t), Some(v)) => {
                out.push_str(v);
                rest = &after[t.len()..];
            }
            _ => {
                out.push('{');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    sanitize_file_name(&out)
}

/// True when both paths refer to the same existing file (handles `.`/`..`,
/// symlinks, hard links and case-insensitive file systems), or are lexically identical.
pub fn is_same_path(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false) || lexical(a) == lexical(b)
}

/// Absolute path with `.` and `..` resolved lexically (no file system access).
fn lexical(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Directory the output goes to (not created here). An empty subfolder means "next to the source".
pub fn output_dir_for(input: &Path, out: &OutputSettings) -> PathBuf {
    let beside = || {
        let parent = input.parent().unwrap_or(Path::new("."));
        let sub = out.subfolder.trim();
        if sub.is_empty() { parent.to_path_buf() } else { parent.join(sanitize_file_name(sub)) }
    };
    match (out.dir, &out.custom_dir) {
        (OutputDir::Custom, Some(dir)) => dir.clone(),
        _ => beside(),
    }
}

/// Desired output path: rendered name in the output dir. Never equal to the
/// input; with `Rename`, also avoids existing files and paths reserved by
/// other queued jobs.
pub fn plan_output_path(
    input: &Path,
    out: &OutputSettings,
    codec: &str,
    mode: &str,
    date: &str,
    reserved: &HashSet<PathBuf>,
) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = render_name(&out.name_template, &stem, codec, mode, date);
    let dir = output_dir_for(input, out);
    plan_named_output(input, &dir, &name, out.container.ext(), out.conflict, reserved)
}

/// Lower-level planner used by every job kind (extraction picks its own extension).
pub fn plan_named_output(
    input: &Path,
    dir: &Path,
    name: &str,
    ext: &str,
    policy: ConflictPolicy,
    reserved: &HashSet<PathBuf>,
) -> PathBuf {
    let desired = dir.join(format!("{name}.{ext}"));
    let taken = |p: &Path| {
        is_same_path(input, p)
            || reserved.contains(p)
            || (policy == ConflictPolicy::Rename && p.symlink_metadata().is_ok())
    };
    if !taken(&desired) {
        return desired;
    }
    (2..=9999).map(|n| numbered(&desired, n)).find(|p| !taken(p)).unwrap_or(desired)
}

/// `name.ext` → `name (n).ext`
pub fn numbered(path: &Path, n: u32) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

/// Temp file next to the final output (same volume → atomic rename).
pub fn temp_path_for(final_path: &Path, job_id: &str) -> PathBuf {
    let id8: String = job_id.chars().filter(|c| c.is_ascii_hexdigit()).take(8).collect::<String>().to_ascii_lowercase();
    let file = final_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    final_path.with_file_name(format!("{file}{TEMP_MARK}{id8:0>8}{TEMP_EXT}"))
}

pub fn is_owned_temp(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy()) else { return false };
    let Some(rest) = name.strip_suffix(TEMP_EXT) else { return false };
    let Some(pos) = rest.rfind(TEMP_MARK) else { return false };
    let id = &rest[pos + TEMP_MARK.len()..];
    pos > 0 && id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Delete a temp file this app created. Refuses anything else.
pub fn remove_owned_temp(path: &Path) -> io::Result<()> {
    if !is_owned_temp(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to delete non-temp file {}", path.display()),
        ));
    }
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Atomically move `temp` to `dest`, failing with `AlreadyExists` if `dest` exists.
pub fn rename_no_replace(temp: &Path, dest: &Path) -> io::Result<()> {
    match sys::rename_excl(temp, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(e) if sys::is_unsupported(&e) => rename_via_link(temp, dest),
        Err(e) => Err(e),
    }
}

/// Fallback for volumes without an exclusive rename: `link()` also refuses to
/// replace. Volumes without hard links (exFAT/FAT) get check-then-rename.
fn rename_via_link(temp: &Path, dest: &Path) -> io::Result<()> {
    match std::fs::hard_link(temp, dest) {
        Ok(()) => {
            let _ = std::fs::remove_file(temp);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) if dest.symlink_metadata().is_ok() => {
            Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination exists"))
        }
        Err(_) => std::fs::rename(temp, dest),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("output already exists: {0}")]
    Skipped(PathBuf),
    #[error("could not commit output: {0}")]
    Io(#[from] io::Error),
}

/// Commit `temp` as `desired` (or a numbered free name with `Rename`). Returns the final path.
pub fn commit(temp: &Path, desired: &Path, policy: ConflictPolicy) -> Result<PathBuf, CommitError> {
    let mut candidate = desired.to_path_buf();
    for n in 2..=9999u32 {
        match rename_no_replace(temp, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => match policy {
                ConflictPolicy::Skip => return Err(CommitError::Skipped(candidate)),
                ConflictPolicy::Rename => candidate = numbered(desired, n),
            },
            Err(e) => return Err(e.into()),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free output name").into())
}

/// Free bytes available to the user on the volume holding `path` (or its nearest existing ancestor).
pub fn available_space(path: &Path) -> Option<u64> {
    let abs = std::path::absolute(path).ok()?;
    let mut p = abs.as_path();
    while p.symlink_metadata().is_err() {
        p = p.parent()?;
    }
    sys::available(p)
}

#[cfg(unix)]
mod sys {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn cstr(p: &Path) -> io::Result<CString> {
        CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))
    }

    #[cfg(target_vendor = "apple")]
    pub fn rename_excl(from: &Path, to: &Path) -> io::Result<()> {
        let (f, t) = (cstr(from)?, cstr(to)?);
        // SAFETY: both pointers are valid NUL-terminated strings for the duration of the call.
        let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
        if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    #[cfg(target_os = "linux")]
    pub fn rename_excl(from: &Path, to: &Path) -> io::Result<()> {
        let (f, t) = (cstr(from)?, cstr(to)?);
        const RENAME_NOREPLACE: libc::c_uint = 1;
        // SAFETY: raw renameat2 syscall with valid NUL-terminated paths.
        let r = unsafe {
            libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), RENAME_NOREPLACE)
        };
        if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    pub fn rename_excl(_from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    pub fn is_unsupported(e: &io::Error) -> bool {
        matches!(e.raw_os_error(), Some(c) if c == libc::ENOTSUP || c == libc::EOPNOTSUPP || c == libc::EINVAL || c == libc::ENOSYS)
    }

    pub fn available(p: &Path) -> Option<u64> {
        let c = cstr(p).ok()?;
        // SAFETY: statvfs fills the zeroed struct; we only read it on success.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
}

#[cfg(windows)]
mod sys {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, MoveFileExW, MOVEFILE_WRITE_THROUGH};

    /// NUL-terminated UTF-16, with the `\\?\` prefix for long absolute paths.
    pub fn wide(p: &Path) -> Vec<u16> {
        let s = p.as_os_str();
        let needs_prefix = s.len() >= 248 && p.is_absolute() && !s.to_string_lossy().starts_with(r"\\?\");
        let mut v: Vec<u16> = Vec::new();
        if needs_prefix {
            let text = s.to_string_lossy();
            if let Some(unc) = text.strip_prefix(r"\\") {
                v.extend(r"\\?\UNC\".encode_utf16());
                v.extend(unc.encode_utf16());
            } else {
                v.extend(r"\\?\".encode_utf16());
                v.extend(s.encode_wide());
            }
        } else {
            v.extend(s.encode_wide());
        }
        v.push(0);
        v
    }

    pub fn rename_excl(from: &Path, to: &Path) -> io::Result<()> {
        let (f, t) = (wide(from), wide(to));
        // No MOVEFILE_REPLACE_EXISTING: fails with ERROR_ALREADY_EXISTS if `to` exists.
        // SAFETY: valid NUL-terminated wide strings.
        let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    pub fn is_unsupported(e: &io::Error) -> bool {
        matches!(e.raw_os_error(), Some(50) | Some(1)) // ERROR_NOT_SUPPORTED, ERROR_INVALID_FUNCTION
    }

    pub fn available(p: &Path) -> Option<u64> {
        let w = wide(p);
        let mut avail: u64 = 0;
        // SAFETY: valid wide path; output pointer to a local u64.
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
        if ok != 0 { Some(avail) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn out_settings(dir: OutputDir, custom: Option<PathBuf>) -> OutputSettings {
        OutputSettings { dir, custom_dir: custom, ..OutputSettings::default() }
    }

    #[test]
    fn sanitize_keeps_unicode_spaces_and_symbols() {
        assert_eq!(sanitize_file_name("旅行 Vlog 第1集 #2 (最终版) & 東京タワー 🎬"), "旅行 Vlog 第1集 #2 (最终版) & 東京タワー 🎬");
    }

    #[test]
    fn sanitize_replaces_characters_illegal_on_windows() {
        assert_eq!(sanitize_file_name("a<b>c:d\"e/f\\g|h?i*j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_file_name("tab\there"), "tab_here");
    }

    #[test]
    fn sanitize_trims_trailing_dots_and_spaces_and_avoids_reserved_names() {
        assert_eq!(sanitize_file_name("name. . "), "name");
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("lpt1"), "_lpt1");
        assert_eq!(sanitize_file_name(""), "output");
    }

    #[test]
    fn sanitize_limits_length_on_a_char_boundary() {
        let long = "视".repeat(150); // 450 bytes
        let s = sanitize_file_name(&long);
        assert!(s.len() <= MAX_NAME_BYTES);
        assert!(s.chars().all(|c| c == '视'));
    }

    #[test]
    fn render_name_expands_tokens() {
        assert_eq!(render_name("{name}_{codec}", "猫 咪", "av1", "crf30", "20261103"), "猫 咪_av1");
        assert_eq!(render_name("{date}-{name}-{mode}", "a", "h264", "lossless", "20261103"), "20261103-a-lossless");
        assert_eq!(render_name("{name}/x", "a", "h264", "", ""), "a_x");
    }

    #[test]
    fn same_path_detects_equivalent_spellings() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("源 文件.mp4");
        fs::write(&f, b"x").unwrap();
        let alt = d.path().join("sub").join("..").join("源 文件.mp4");
        fs::create_dir(d.path().join("sub")).unwrap();
        assert!(is_same_path(&f, &alt));
        assert!(!is_same_path(&f, &d.path().join("other.mp4")));
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn same_path_detects_case_only_difference_on_case_insensitive_fs() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("Clip.MP4");
        fs::write(&f, b"x").unwrap();
        assert!(is_same_path(&f, &d.path().join("clip.mp4")));
    }

    #[test]
    fn output_dir_beside_source_uses_subfolder() {
        let o = out_settings(OutputDir::BesideSource, None);
        assert_eq!(output_dir_for(Path::new("/v/a.mp4"), &o), PathBuf::from("/v").join("FrameEase 输出"));
        let c = out_settings(OutputDir::Custom, Some(PathBuf::from("/out")));
        assert_eq!(output_dir_for(Path::new("/v/a.mp4"), &c), PathBuf::from("/out"));
    }

    #[test]
    fn numbered_inserts_counter_before_extension() {
        assert_eq!(numbered(Path::new("/o/视频.mp4"), 2), PathBuf::from("/o/视频 (2).mp4"));
    }

    #[test]
    fn plan_never_targets_the_input_file() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("clip.mp4");
        fs::write(&input, b"src").unwrap();
        let mut o = out_settings(OutputDir::Custom, Some(d.path().to_path_buf()));
        o.name_template = "{name}".into();
        for policy in [ConflictPolicy::Rename, ConflictPolicy::Skip] {
            o.conflict = policy;
            let p = plan_output_path(&input, &o, "h264", "", "", &HashSet::new());
            assert!(!is_same_path(&input, &p), "{policy:?} planned the input itself");
            assert_eq!(p, d.path().join("clip (2).mp4"));
        }
    }

    #[test]
    fn plan_avoids_existing_and_reserved_names_with_rename_policy() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("in").join("clip.mov");
        let o = out_settings(OutputDir::Custom, Some(d.path().to_path_buf()));
        fs::write(d.path().join("clip_h264.mp4"), b"old").unwrap();
        let mut reserved = HashSet::new();
        reserved.insert(d.path().join("clip_h264 (2).mp4"));
        let p = plan_output_path(&input, &o, "h264", "", "", &reserved);
        assert_eq!(p, d.path().join("clip_h264 (3).mp4"));
    }

    #[test]
    fn temp_names_are_owned_and_recognisable() {
        let t = temp_path_for(Path::new("/o/旅行_av1.mp4"), "0123456789abcdef");
        assert_eq!(t, PathBuf::from("/o/旅行_av1.mp4.fe-01234567.part"));
        assert!(is_owned_temp(&t));
        assert!(!is_owned_temp(Path::new("/o/旅行_av1.mp4")));
        assert!(!is_owned_temp(Path::new("/o/movie.part")));
        assert!(!is_owned_temp(Path::new("/o/a.fe-zz.part")));
    }

    #[test]
    fn remove_owned_temp_refuses_to_delete_a_source_video() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("原片.mp4");
        fs::write(&src, b"precious").unwrap();
        assert!(remove_owned_temp(&src).is_err());
        assert!(src.exists(), "source must survive");
        let t = temp_path_for(&d.path().join("out.mp4"), "abcdef0123456789");
        fs::write(&t, b"partial").unwrap();
        remove_owned_temp(&t).unwrap();
        assert!(!t.exists());
        remove_owned_temp(&t).unwrap(); // already gone is fine
    }

    #[test]
    fn rename_no_replace_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let temp = d.path().join("x.mp4.fe-00000000.part");
        let dest = d.path().join("x.mp4");
        fs::write(&temp, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();
        let err = rename_no_replace(&temp, &dest).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert_eq!(fs::read(&temp).unwrap(), b"new");
        fs::remove_file(&dest).unwrap();
        rename_no_replace(&temp, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert!(!temp.exists());
    }

    #[test]
    fn commit_renames_around_existing_output() {
        let d = tempfile::tempdir().unwrap();
        let desired = d.path().join("结果.mkv");
        fs::write(&desired, b"old").unwrap();
        let temp = temp_path_for(&desired, "1111111122222222");
        fs::write(&temp, b"new").unwrap();
        let final_path = commit(&temp, &desired, ConflictPolicy::Rename).unwrap();
        assert_eq!(final_path, d.path().join("结果 (2).mkv"));
        assert_eq!(fs::read(&desired).unwrap(), b"old");
        assert_eq!(fs::read(&final_path).unwrap(), b"new");
    }

    #[test]
    fn commit_with_skip_policy_leaves_existing_output_alone() {
        let d = tempfile::tempdir().unwrap();
        let desired = d.path().join("a.mp4");
        fs::write(&desired, b"old").unwrap();
        let temp = temp_path_for(&desired, "1111111122222222");
        fs::write(&temp, b"new").unwrap();
        assert!(matches!(commit(&temp, &desired, ConflictPolicy::Skip), Err(CommitError::Skipped(_))));
        assert_eq!(fs::read(&desired).unwrap(), b"old");
    }

    #[test]
    fn available_space_reports_something_for_missing_subdirs() {
        let d = tempfile::tempdir().unwrap();
        let space = available_space(&d.path().join("not").join("yet"));
        assert!(space.unwrap_or(0) > 0);
    }
}
