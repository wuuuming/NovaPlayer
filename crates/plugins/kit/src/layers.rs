//! Lyrics layers (docs/extensions.md, "Lyrics layers"): extra lines a plugin's `metadata` gives under the
//! lyrics, as `"layers": { "<id>": { "name": "…", "lrc": "<LRC of the same stamps>" } }`.

use crate::json::{self, Value};

/// A lyrics text is an LRC (it holds a `[` stamp): a layer worth giving.
pub fn is_lrc(text: &str) -> bool {
    text.contains('[')
}

/// The `layers` value holding one translation (id `trans`, named in the player's language).
pub fn translation(lrc: &str) -> Value {
    layers(Some(lrc), None)
}

/// The `layers` value: a translation (`trans`) and a romanisation (`roma`), each when there is
/// one, named in the player's language.
pub fn layers(trans: Option<&str>, roma: Option<&str>) -> Value {
    let one = |name: &str, lrc: &str| json::obj(vec![("name", Value::str(name)), ("lrc", Value::str(lrc))]);
    let mut v = Vec::new();
    if let Some(t) = trans {
        v.push(("trans", one(crate::pipe::tr("翻译", "Translation"), t)));
    }
    if let Some(r) = roma {
        v.push(("roma", one(crate::pipe::tr("罗马音", "Romanization"), r)));
    }
    json::obj(v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_translation_layer() {
        let v = super::translation("[00:01.00]你好");
        assert_eq!(crate::json::write(&v), r#"{"trans":{"lrc":"[00:01.00]你好","name":"Translation"}}"#);
    }
}
