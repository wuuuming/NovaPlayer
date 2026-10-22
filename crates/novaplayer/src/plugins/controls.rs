//! What plugins add to the player's face (docs/extensions.md): controls, each a `.nui` fragment
//! from the plugin's folder mounted in one of the player's slots while its condition holds.
//!
//! The player owns where and how big (the slot), what a control may read (the context keys) and
//! what a press may do (the commands). The fragment owns only its looks: the framework's
//! `parse_fragment` admits the elements and properties of `PROFILE`, nothing else.

pub use crate::views::plugin_controls::Mounts;
use novaplayer_plugin_protocol::json;
use novaui::dsl::{Cond, Profile, Sheet, Val};
use std::path::Path;

/// The elements and properties a plugin's fragment may use: looks only. No position, size,
/// layer or z order (the slot decides), no bindings to the player's own names.
pub const PROFILE: Profile = Profile {
    kinds: &["Row", "Column", "Stack", "Box", "Text", "Path", "Button", "Spacer"],
    props: &[
        "text",
        "font-size",
        "weight",
        "line",
        "fg",
        "bg",
        "radius",
        "gap",
        "padding",
        "align",
        "justify",
        "lit",
        "disabled",
        "on-click",
        "shown",
        "view",
        "d",
        "icon-size",
        "opacity",
        "border",
        "cursor",
        "hit",
    ],
    max_bytes: 64 * 1024,
    max_nodes: 256,
};

/// The slots a control may name, and the player's styles a control there may take.
pub const SLOTS: &[(&str, &[&str])] = &[("lyrics-tools", &["ctl"])];

/// The commands a fragment's `on-click` may name (besides `plugin.<name>`).
const COMMANDS: &[&str] = &["option.open"];

/// One control a plugin's manifest declares.
#[derive(Clone, Debug)]
pub struct Control {
    pub id: String,
    pub slot: String,
    pub component: String,
    /// When it shows: a condition over the context keys, checked when the plugin was scanned
    /// (`shows` reads it).
    when: Option<String>,
    /// The fragment's source, read and checked when the plugin was scanned (parsed again where
    /// it mounts, `sheet`: a parsed sheet stays on the UI thread, the scan need not).
    pub src: String,
    /// Whose `@key` texts it reads (the plugin's id).
    keys: Option<String>,
    /// The context keys the fragment reads (`$song.layers.trans`…): pushed to it.
    pub reads: Vec<String>,
}

impl Control {
    /// Whether it shows where the context keys read as `get` says (always, without a `when`).
    pub fn shows(&self, get: &dyn Fn(&str) -> Option<Val>) -> bool {
        self.when.as_deref().is_none_or(|w| Cond::parse(w).is_ok_and(|c| c.eval(get)))
    }

    /// Its fragment, parsed as the scan checked it.
    pub fn sheet(&self) -> Result<Sheet, String> {
        novaui::dsl::parse_fragment(&self.src, &PROFILE, self.keys.as_deref()).map_err(|e| e.to_string())
    }
}

impl PartialEq for Control {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.slot == o.slot && self.component == o.component
    }
}

/// `contributes.controls` of a manifest, each fragment read from the plugin folder. Any fault is
/// the plugin's to show (`Err`): its other capabilities are kept.
pub fn read_controls(dir: &Path, v: &json::Value) -> Result<Vec<Control>, String> {
    let Some(list) = v.get("contributes").and_then(|c| c.get("controls")).and_then(json::Value::as_array) else {
        return Ok(Vec::new());
    };
    // Its fragments' `@key` texts are the plugin's own strings (`id:key`, `plugins.md`).
    let keys = v.get("id").and_then(json::Value::as_str);
    let mut out = Vec::new();
    for c in list {
        let s = |k: &str| c.get(k).and_then(json::Value::as_str).map(str::to_owned);
        let id = s("id").ok_or("a control has no id")?;
        let slot = s("slot").ok_or_else(|| format!("control {id}: no slot"))?;
        if !SLOTS.iter().any(|(n, _)| *n == slot) {
            return Err(format!("control {id}: no slot named {slot}"));
        }
        let file = s("ui").ok_or_else(|| format!("control {id}: no ui"))?;
        if !crate::plugins::registry::inside_folder(&file) {
            return Err(format!("control {id}: ui must be a relative path inside the plugin folder"));
        }
        let path = dir.join(&file);
        let meta = std::fs::metadata(&path).map_err(|_| format!("control {id}: {file} not found"))?;
        if meta.len() as usize > PROFILE.max_bytes {
            return Err(format!("control {id}: {file} is too large"));
        }
        let src = std::fs::read_to_string(&path).map_err(|_| format!("control {id}: {file} cannot be read"))?;
        let sheet =
            novaui::dsl::parse_fragment(&src, &PROFILE, keys).map_err(|e| format!("control {id}: {file} {e}"))?;
        let component = s("component").unwrap_or_else(|| sheet.component_names().next().unwrap_or_default().to_owned());
        if !sheet.has_component(&component) {
            return Err(format!("control {id}: {file} has no component {component}"));
        }
        check_actions(&sheet).map_err(|a| format!("control {id}: unsupported command {a}"))?;
        let when = match s("when") {
            Some(w) => {
                Cond::parse(&w).map_err(|e| format!("control {id}: when cannot be parsed: {e}"))?;
                Some(w)
            }
            None => None,
        };
        let reads = sheet.reads();
        out.push(Control { id, slot, component, when, src, keys: keys.map(str::to_owned), reads });
    }
    Ok(out)
}

/// Every `on-click` names a known command (with its argument) or `plugin.<name>`, wherever the
/// fragment writes it (its styles too: `Sheet::actions`).
fn check_actions(sheet: &Sheet) -> Result<(), String> {
    for a in sheet.actions() {
        let verb = a.split_whitespace().next().unwrap_or("");
        let ok = COMMANDS.contains(&verb) || verb.strip_prefix("plugin.").is_some_and(|n| !n.is_empty());
        if !ok {
            return Err(a);
        }
    }
    Ok(())
}

/// The player's state a control may read (docs/extensions.md, "Context keys").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub page: &'static str,
    pub playing: bool,
    /// The playing song's plugin ("" for a local song).
    pub song_plugin: String,
    /// The layers the playing song's lyrics come with.
    pub layers: Vec<String>,
}

impl Context {
    /// A key's value for a control of `plugin`.
    pub fn get(&self, plugin: &str, key: &str) -> Option<Val> {
        Some(match key {
            "page" => Val::Str(self.page.to_owned()),
            "playing" => Val::Bool(self.playing),
            "song.plugin" => Val::Str(self.song_plugin.clone()),
            "self" => Val::Str(plugin.to_owned()),
            k => {
                if let Some(l) = k.strip_prefix("song.layers.") {
                    Val::Bool(self.layers.iter().any(|x| x == l))
                } else {
                    return None;
                }
            }
        })
    }
}

/// A control's press: `contrib:<plugin>:<command> <arg>`.
pub fn parse_action(a: &str) -> Option<(&str, &str, &str)> {
    let rest = a.strip_prefix("contrib:")?;
    let (plugin, cmd) = rest.split_once(':')?;
    let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
    Some((plugin, verb, arg.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> std::path::PathBuf {
        // One directory per call: tests run in parallel and each removes its own first.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("np-contrib-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (n, t) in files {
            let p = d.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        d
    }

    const FRAG: &str = r#"
component Trans() {
  Button .ctl { lit: $song.layers.trans; on-click: "option.open";
    Path { view: 24; icon-size: 18; d: "M3 5H11V7H3Z"; }
  }
}
"#;

    #[test]
    fn a_manifest_control_reads_its_fragment_and_condition() {
        let d = dir_with(&[("ui/c.nui", FRAG)]);
        let v = json::parse(
            r#"{"contributes":{"controls":[{"id":"trans","slot":"lyrics-tools","ui":"ui/c.nui","component":"Trans",
                 "when":"song.plugin == self && song.layers.trans && page == \"lyrics\""}]}}"#,
        )
        .unwrap();
        let cs = read_controls(&d, &v).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].reads, vec!["song.layers.trans".to_string()]);
        let mut ctx =
            Context { page: "lyrics", song_plugin: "svc".into(), layers: vec!["trans".into()], ..Default::default() };
        let w = &cs[0];
        assert!(w.shows(&|k| ctx.get("svc", k)));
        assert!(!w.shows(&|k| ctx.get("other", k)), "another plugin's song");
        ctx.page = "queue";
        assert!(!w.shows(&|k| ctx.get("svc", k)));
        ctx.page = "lyrics";
        ctx.layers.clear();
        assert!(!w.shows(&|k| ctx.get("svc", k)), "a song without the layer");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn faults_are_refused_with_a_reason() {
        let cases = [
            (r#"{"id":"a","slot":"nowhere","ui":"c.nui"}"#, FRAG, "no slot named"),
            (r#"{"id":"a","slot":"lyrics-tools","ui":"../c.nui"}"#, FRAG, "relative path"),
            (r#"{"id":"a","slot":"lyrics-tools","ui":"c.nui"}"#, "component A() { Box { left: 3; } }", "line"),
            (
                r#"{"id":"a","slot":"lyrics-tools","ui":"c.nui"}"#,
                "component A() { Button { on-click: quit; } }",
                "unsupported command",
            ),
            // A style's press is a press: it names only what an element's may (regression: the
            // check walked the components alone).
            (
                r#"{"id":"a","slot":"lyrics-tools","ui":"c.nui"}"#,
                "style s { on-click: \"plugin-delete:x\"; } component A() { Button .s { } }",
                "unsupported command",
            ),
            (r#"{"id":"a","slot":"lyrics-tools","ui":"c.nui","when":"page =="}"#, FRAG, "when"),
        ];
        for (c, frag, why) in cases {
            let d = dir_with(&[("c.nui", frag)]);
            let v = json::parse(&format!(r#"{{"contributes":{{"controls":[{c}]}}}}"#)).unwrap();
            let e = read_controls(&d, &v).unwrap_err();
            assert!(e.contains(why), "{c}: {e}");
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn actions_name_their_plugin() {
        assert_eq!(parse_action("contrib:svc:option.open"), Some(("svc", "option.open", "")));
        assert_eq!(parse_action("contrib:svc:plugin.like"), Some(("svc", "plugin.like", "")));
        assert_eq!(parse_action("play"), None);
    }

    /// The plugins' own control files, as shipped, pass the player's profile.
    #[test]
    fn shipped_plugin_controls_are_valid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        for dir in std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()) {
            let Ok(manifest) = std::fs::read_to_string(dir.join("plugin.json")) else { continue };
            let v = json::parse(&manifest).unwrap();
            read_controls(&dir, &v).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        }
    }
}
