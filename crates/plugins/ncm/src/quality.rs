//! NetEase quality IDs, display names and resolution labels.
/// NetEase's levels, highest first: (level for song_url_v1, the app's own name, its English
/// name, the key in song/music/detail/get). 沉浸环绕声 (sky) and 杜比全景声 are left out: multichannel streams.
pub const LEVELS: [(&str, &str, &str, &str); 7] = [
    ("jymaster", "超清母带", "Hi-Res Master", "jm"),
    ("jyeffect", "高清臻音", "Spatial HD", "je"),
    ("hires", "高解析度无损", "Hi-Res lossless", "hr"),
    ("lossless", "无损", "Lossless", "sq"),
    ("exhigh", "极高", "Very high", "h"),
    ("higher", "较高", "Higher", "m"),
    ("standard", "标准", "Standard", "l"),
];
/// Without a choice: 无损 (SQ).
pub const DEFAULT_LEVEL: usize = 3;

pub fn level_name(level: &str) -> &'static str {
    LEVELS.iter().find(|l| l.0 == level).map_or("", |l| kit::pipe::tr(l.1, l.2))
}

/// What a level is, in the player's own words (player-core AudioResolution::label): lossless
/// "48 kHz / 24-bit FLAC", lossy "48 kHz MP3 320 kbps".
pub fn quality_label(level: &str, sr: u32, br: u32) -> String {
    let rate = if sr == 0 {
        String::new()
    } else if sr.is_multiple_of(1000) {
        format!("{} kHz", sr / 1000)
    } else {
        format!("{:.1} kHz", f64::from(sr) / 1000.0)
    };
    match level {
        "lossless" => format!("{rate} / 16-bit FLAC"),
        "hires" | "jyeffect" | "jymaster" => format!("{rate} / 24-bit FLAC"),
        _ => format!("{rate} MP3 {} kbps", (br + 500) / 1000),
    }
    .trim()
    .to_owned()
}
