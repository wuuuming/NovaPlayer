//! The extended M3U8 a plugin's `playlist` answers with (docs/plugins.md §playlist): the list's
//! name, and for each song its length, artist, title and album, then its URI.

pub struct Entry<'a> {
    pub uri: &'a str,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    /// Seconds; `None` is written as -1 (unknown).
    pub duration: Option<f64>,
}

/// One line's words: a line break or a comma inside would end the field early.
fn clean(s: &str) -> String {
    s.replace(['\r', '\n'], " ").trim().to_owned()
}

pub fn write<'a>(name: &str, entries: impl IntoIterator<Item = Entry<'a>>) -> String {
    let mut out = format!("#EXTM3U\n#PLAYLIST:{}\n", clean(name));
    for e in entries {
        let secs = e.duration.filter(|d| *d > 0.0).map_or(-1, |d| d.round() as i64);
        let (artist, title) = (clean(e.artist), clean(e.title));
        let words = if artist.is_empty() { title } else { format!("{artist} - {title}") };
        out.push_str(&format!("#EXTINF:{secs},{words}\n"));
        let album = clean(e.album);
        if !album.is_empty() {
            out.push_str(&format!("#EXTALB:{album}\n"));
        }
        out.push_str(&clean(e.uri));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_extended_m3u8() {
        let t = super::write(
            "我喜欢\n的",
            [super::Entry {
                uri: "ncm:song:1",
                title: "海屿你",
                artist: "马也",
                album: "海屿你",
                duration: Some(215.4),
            }],
        );
        assert_eq!(t, "#EXTM3U\n#PLAYLIST:我喜欢 的\n#EXTINF:215,马也 - 海屿你\n#EXTALB:海屿你\nncm:song:1\n");
    }
}
