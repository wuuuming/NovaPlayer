//! NetEase lyrics: the word-by-word (`yrc`) text of `song/lyric/v1`, turned into the enhanced
//! LRC the plugin protocol's `metadata.lyrics` carries (`docs/plugins.md`).
//!
//! YRC: a line is `[start,duration]` (ms) and each of its words `(start,duration,0)word`
//! (AMLL's yrc.ts): `[190871,1984](190871,361,0)For (191232,172,0)the …`. Enhanced LRC stamps
//! a word's start before it and the line's end after the last one:
//! `[03:10.87]<03:10.87>For <03:11.23>the <03:12.85>`.

use crate::api::Client;
use kit::json::{self, Value};

impl Client {
    /// The song's lyrics, word by word where the service has them (song/lyric/v1: `yrc`), and
    /// the translation / romanisation layers of the same stamps. The `lyrics` and `layers`
    /// fields of the plugin's `metadata` answer; empty when the song has none.
    pub fn lyrics(&mut self, id: u64, token: Option<&str>) -> Vec<(String, Value)> {
        let data = json::obj(vec![
            ("id", Value::Num(id as f64)),
            ("cp", Value::Bool(false)),
            ("tv", Value::Num(0.0)),
            ("lv", Value::Num(0.0)),
            ("rv", Value::Num(0.0)),
            ("kv", Value::Num(0.0)),
            ("yv", Value::Num(0.0)),
            ("ytv", Value::Num(0.0)),
            ("yrv", Value::Num(0.0)),
        ]);
        let Ok(a) = self.call("song/lyric/v1", data, token) else { return Vec::new() };
        if a.body.get("nolyric").and_then(Value::as_bool).unwrap_or(false)
            || a.body.get("uncollected").and_then(Value::as_bool).unwrap_or(false)
        {
            return Vec::new();
        }
        let text = |k: &str| a.body.at(&[k, "lyric"]).and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();
        // The word-by-word text wins whole where the song has it; the plain LRC only where it
        // does not. The two are never merged (see `prefer`).
        let lyrics = prefer(yrc_to_lrc(text("yrc")), text("lrc"));
        if lyrics.trim().is_empty() {
            return out;
        }
        out.push(("lyrics".into(), Value::str(lyrics)));
        let layer = |k: &str| {
            let t = text(k);
            kit::layers::is_lrc(t).then_some(t)
        };
        // Prefer the layers aligned to YRC. These normally already use ordinary LRC stamps;
        // accept millisecond line headers too. Fall back to the classic layers if absent.
        let trans = yrc_layer(text("ytlrc")).or_else(|| layer("tlyric").map(str::to_owned));
        let roma = yrc_layer(text("yromalrc")).or_else(|| layer("romalrc").map(str::to_owned));
        if trans.is_some() || roma.is_some() {
            out.push(("layers".into(), kit::layers::layers(trans.as_deref(), roma.as_deref())));
        }
        out
    }
}

/// The lyrics to ship: the word-by-word text when the song has it, else the plain LRC. Never a
/// merge of the two: the same sung line carries different stamps in them, so a merged pair
/// would show the line twice — the plain copy after the word-timed one, without the
/// translation and romanisation (the layers carry the yrc lines' stamps).
fn prefer(yrc: Option<String>, plain: &str) -> String {
    yrc.unwrap_or_else(|| plain.to_owned())
}

/// YRC-aligned translation / romanisation normally arrives as ordinary LRC. Preserve those
/// stamps; also accept `[start,duration]text` by converting its millisecond start to LRC.
fn yrc_layer(text: &str) -> Option<String> {
    let mut out = String::new();
    for line in text.lines() {
        if let Some((start, _dur, rest)) = line_head(line.trim()) {
            out.push_str(&format!("[{}]{}\n", stamp(start), rest));
        } else if line.trim_start().starts_with('[') {
            out.push_str(line);
            out.push('\n');
        }
    }
    kit::layers::is_lrc(&out).then_some(out)
}

/// A YRC text as enhanced LRC: the line's `[start,duration]` becomes its `[mm:ss.xx]` stamp and
/// each `(start,duration,0)word` a `<start>` before its word; a stamp after the last word is the
/// line's end. Lines that are not timed words (the JSON credit headers) are left out.
/// `None` when nothing was timed word by word.
pub fn yrc_to_lrc(yrc: &str) -> Option<String> {
    let mut out = String::new();
    for line in yrc.lines() {
        let line = line.trim();
        let Some((start, dur, rest)) = line_head(line) else { continue };
        // A `(start,duration,0)` is a word's start; any other '(' (a sung "(oh)") is text of
        // the word before it.
        let mut stamps: Vec<(usize, usize, u64, u64)> = Vec::new(); // ('(' byte, text from, start, dur)
        let mut i = 0;
        while let Some(p) = rest[i..].find('(') {
            let p = i + p;
            let after = &rest[p + 1..];
            let Some(close) = after.find(')') else { break };
            match word_stamp(&after[..close]) {
                Some((at, d)) => {
                    stamps.push((p, p + 1 + close + 1, at, d));
                    i = p + 1 + close + 1;
                }
                None => i = p + 1,
            }
        }
        if stamps.is_empty() {
            continue;
        }
        out.push_str(&format!("[{}]", stamp(start)));
        let mut end = start + dur;
        for (k, &(_, from, at, d)) in stamps.iter().enumerate() {
            let to = stamps.get(k + 1).map(|&(p, ..)| p).unwrap_or(rest.len());
            out.push_str(&format!("<{}>{}", stamp(at), &rest[from..to]));
            end = end.max(at + d);
        }
        // The last stamp ends the line: the line's own end, else its last word's.
        out.push_str(&format!("<{}>\n", stamp(end)));
    }
    (!out.is_empty()).then_some(out)
}

/// `[start,duration]` at a yrc line's head (ms). Any other line (a JSON credit header) is none.
fn line_head(line: &str) -> Option<(u64, u64, &str)> {
    let rest = line.strip_prefix('[')?;
    let (tag, after) = rest.split_once(']')?;
    let (start, dur) = tag.split_once(',')?;
    Some((start.trim().parse().ok()?, dur.trim().parse().ok()?, after))
}

/// `(start,duration,0)`: the word's start (ms).
fn word_stamp(tag: &str) -> Option<(u64, u64)> {
    let mut parts = tag.splitn(3, ',');
    let start = parts.next()?.trim().parse().ok()?;
    let dur = parts.next()?.trim().parse().ok()?;
    (parts.next().unwrap_or("0").trim() == "0").then_some((start, dur))
}

/// `[mm:ss.xx]` (centiseconds, as LRC writes them) of a millisecond time, rounded.
fn stamp(ms: u64) -> String {
    let cs = (ms + 5) / 10;
    format!("{:02}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yrc_becomes_word_stamped_lrc() {
        let yrc = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"},{\"tx\":\"Someone\"}]}\n\
                   [190871,1984](190871,361,0)For (191232,172,0)the (191404,376,0)first (191780,1075,0)time\n\
                   [193459,4198](193459,412,0)What's (193871,574,0)past (194445,506,0)is (194951,2706,0)past\n";
        let lrc = yrc_to_lrc(yrc).unwrap();
        assert_eq!(
            lrc,
            "[03:10.87]<03:10.87>For <03:11.23>the <03:11.40>first <03:11.78>time<03:12.86>\n\
             [03:13.46]<03:13.46>What's <03:13.87>past <03:14.45>is <03:14.95>past<03:17.66>\n"
        );
    }

    #[test]
    fn a_line_without_word_stamps_is_dropped() {
        let yrc = "[00:01.00]纯音乐，请欣赏\n[10000,2000](10000,500,0)la\n";
        assert_eq!(yrc_to_lrc(yrc).unwrap(), "[00:10.00]<00:10.00>la<00:12.00>\n");
        assert_eq!(yrc_to_lrc("[00:01.00]only lines\n"), None);
    }

    #[test]
    fn yrc_layers_accept_normal_lrc_with_yrc_aligned_times() {
        // The service's word-timed layers use ordinary LRC stamps, not [start,duration].
        // The classic layers can have different times and must not replace these.
        let trans = "[00:20.210]translation\n[00:24.540]next translation\n";
        let roma = "[00:20.210]do u shi ta tte fu re na i \n";
        assert_eq!(yrc_layer(trans).as_deref(), Some(trans));
        assert_eq!(yrc_layer(roma).as_deref(), Some(roma));
    }

    #[test]
    fn a_yrc_translation_layer_keeps_its_line_times() {
        let t = "[190871,1984]第一次\n[193459,4198]过去就过去了\n";
        assert_eq!(yrc_layer(t).unwrap(), "[03:10.87]第一次\n[03:13.46]过去就过去了\n");
        assert_eq!(yrc_layer(""), None);
    }

    #[test]
    fn the_plain_lrc_is_not_merged_into_the_word_timed_text() {
        // The same line's stamps differ between lrc and yrc: merging would show it twice, the
        // plain copy without its layers. The word-timed text ships alone.
        let plain = "[00:01.00]one\n[00:05.07]two\n[00:09.00]three\n";
        let yrc = "[00:05.00]<00:05.00>two <00:06.00>\n".to_owned();
        assert_eq!(prefer(Some(yrc.clone()), plain), yrc);
        assert_eq!(prefer(None, plain), plain);
    }

    #[test]
    fn a_word_with_parentheses_keeps_its_text() {
        // A word's text may hold a '(' that is not a stamp: the stamp is the last '(' before it.
        let yrc = "[1000,1500](1000,400,0)(oh) (1400,600,0)yeah\n";
        assert_eq!(yrc_to_lrc(yrc).unwrap(), "[00:01.00]<00:01.00>(oh) <00:01.40>yeah<00:02.50>\n");
    }
}
