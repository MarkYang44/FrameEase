//! Presets: built-ins plus user presets that can be saved, imported and exported.

use crate::settings::*;
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "frameease-preset";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub settings: JobSettings,
    pub builtin: bool,
}

/// On-disk exchange format for one preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub settings: JobSettings,
}

fn preset(id: &str, name: &str, f: impl FnOnce(&mut JobSettings)) -> Preset {
    let mut settings = JobSettings::default();
    f(&mut settings);
    Preset { id: id.into(), name: name.into(), settings, builtin: true }
}

pub fn builtin_presets() -> Vec<Preset> {
    vec![
        preset("builtin-h264-quality", "H.264 · 高画质", |s| s.video.level = 25),
        preset("builtin-h264-balanced", "H.264 · 均衡（兼容性最好）", |_| {}),
        preset("builtin-hevc-balanced", "HEVC · 均衡", |s| s.video.codec = VideoCodec::Hevc),
        preset("builtin-av1-small", "AV1 · 小体积", |s| {
            s.video.codec = VideoCodec::Av1;
            s.video.level = 75;
        }),
        preset("builtin-h264-gpu", "H.264 · GPU 极速", |s| s.video.speed = Speed::Gpu),
        preset("builtin-ffv1-archive", "FFV1 · 无损存档（MKV）", |s| {
            s.video.codec = VideoCodec::Ffv1;
            s.video.mode = VideoMode::Lossless;
            s.output.container = Container::Mkv;
        }),
        preset("builtin-remux-mkv", "仅重新封装为 MKV（不重编码）", |s| {
            s.video.mode = VideoMode::Copy;
            s.output.container = Container::Mkv;
            s.output.name_template = "{name}".into();
        }),
    ]
}

pub fn export_json(p: &Preset) -> String {
    let file = PresetFile { format: FORMAT.into(), version: VERSION, name: p.name.clone(), settings: p.settings.clone() };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// Parse an exported preset. Unknown fields are ignored, missing ones get defaults,
/// values are clamped; machine-specific output folders are dropped.
pub fn import_json(text: &str) -> Result<Preset, String> {
    let file: PresetFile = serde_json::from_str(text).map_err(|e| format!("not a preset file: {e}"))?;
    if file.format != FORMAT {
        return Err(format!("unexpected format {:?}", file.format));
    }
    if file.version == 0 || file.version > VERSION {
        return Err(format!("unsupported preset version {}", file.version));
    }
    let mut settings = file.settings;
    settings.video.level = settings.video.level.min(100);
    if settings.output.dir == OutputDir::Custom {
        settings.output.dir = OutputDir::BesideSource;
        settings.output.custom_dir = None;
    }
    settings.subtitles.external.clear();
    if let Some(BurnSource::External { .. }) = settings.subtitles.burn {
        settings.subtitles.burn = None;
        settings.subtitles.mode = SubtitleMode::Keep;
    }
    settings.trim = None;
    let name = if file.name.trim().is_empty() { "导入的预设".to_string() } else { file.name.trim().chars().take(60).collect() };
    Ok(Preset { id: uuid::Uuid::new_v4().to_string(), name, settings, builtin: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_cover_quality_size_lossless_and_remux() {
        let b = builtin_presets();
        assert!(b.iter().all(|p| p.builtin && !p.name.is_empty()));
        assert!(b.iter().any(|p| p.settings.video.mode == VideoMode::Lossless && p.settings.video.codec == VideoCodec::Ffv1));
        assert!(b.iter().any(|p| p.settings.video.mode == VideoMode::Copy));
        assert!(b.iter().any(|p| p.settings.video.codec == VideoCodec::Av1));
        let ids: std::collections::HashSet<_> = b.iter().map(|p| &p.id).collect();
        assert_eq!(ids.len(), b.len(), "unique ids");
    }

    #[test]
    fn export_import_roundtrip_keeps_settings_and_name() {
        let mut p = builtin_presets().remove(0);
        p.name = "我的 AV1 预设".into();
        p.settings.video.level = 63;
        let back = import_json(&export_json(&p)).unwrap();
        assert_eq!(back.name, "我的 AV1 预设");
        assert_eq!(back.settings.video, p.settings.video);
        assert!(!back.builtin);
        assert_ne!(back.id, p.id, "imported presets get a fresh id");
    }

    #[test]
    fn import_rejects_other_files() {
        assert!(import_json("{\"format\":\"something-else\",\"version\":1,\"name\":\"x\",\"settings\":{}}").is_err());
        assert!(import_json("not json").is_err());
        assert!(import_json("{\"format\":\"frameease-preset\",\"version\":99,\"name\":\"x\",\"settings\":{}}").is_err());
    }

    #[test]
    fn import_fills_missing_fields_and_clamps_values() {
        let p = import_json(r#"{"format":"frameease-preset","version":1,"name":"  ","settings":{"video":{"codec":"hevc","level":250},"output":{"dir":"custom","customDir":"/Users/someone/else"}}}"#).unwrap();
        assert_eq!(p.settings.video.codec, VideoCodec::Hevc);
        assert_eq!(p.settings.video.level, 100);
        assert_eq!(p.settings.audio, AudioSettings::default());
        assert_eq!(p.settings.output.dir, OutputDir::BesideSource, "foreign folders are not imported");
        assert_eq!(p.settings.output.custom_dir, None);
        assert_eq!(p.name, "导入的预设");
    }
}
