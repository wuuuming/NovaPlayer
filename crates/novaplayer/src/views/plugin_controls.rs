//! View ownership of plugin fragments. The protocol parser and context live in `plugins`;
//! only this view mounts fragments and writes their bindings.
use crate::plugins::controls::{Context, Control, SLOTS};
use novaui::dsl::{Doc, Mount};
use novaui::{Id, Ui};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Mounts {
    on: BTreeMap<(String, String), Mount>,
    last: Option<Context>,
    reset: bool,
}

impl Mounts {
    /// The plugins' controls changed (a reload, a plugin turned on): the next `sync` mounts
    /// them all again.
    pub fn clear(&mut self) {
        self.reset = true;
    }

    /// Mounts controls whose conditions hold and pushes the keys each reads. A context equal to
    /// the last one, with no `clear` since, leaves everything as it is; only otherwise are the
    /// plugins' controls taken (`plugins`, a copy of the registry's parsed fragments).
    pub fn sync(
        &mut self,
        ui: &mut Ui,
        doc: &mut Doc,
        plugins: &dyn Fn() -> Vec<(String, Vec<Control>)>,
        ctx: &Context,
        slot_of: &dyn Fn(&str) -> Option<Id>,
    ) {
        if !self.reset && self.last.as_ref() == Some(ctx) {
            return;
        }
        let plugins = plugins();
        if std::mem::take(&mut self.reset) {
            for (_, m) in std::mem::take(&mut self.on) {
                doc.unmount(ui, &m);
            }
        }
        self.last = Some(ctx.clone());
        let mut want: Vec<(String, &Control)> = Vec::new();
        for (plugin, cs) in &plugins {
            for c in cs {
                if c.shows(&|k| ctx.get(plugin, k)) {
                    want.push((plugin.clone(), c));
                }
            }
        }
        let keys: Vec<(String, String)> = self.on.keys().cloned().collect();
        for k in keys {
            if !want.iter().any(|(p, c)| p == &k.0 && c.id == k.1) {
                if let Some(m) = self.on.remove(&k) {
                    doc.unmount(ui, &m);
                }
            }
        }
        for (plugin, c) in want {
            let key = (plugin.clone(), c.id.clone());
            if !self.on.contains_key(&key) {
                let Some(slot) = slot_of(&c.slot) else { continue };
                let base = SLOTS.iter().find(|(n, _)| *n == c.slot).map_or(&[][..], |(_, b)| *b);
                let prefix = format!("contrib:{plugin}:");
                let mounted = c.sheet().and_then(|sheet| {
                    doc.mount(ui, slot, &sheet, &c.component, base, &prefix).map_err(|e| e.to_string())
                });
                match mounted {
                    Ok(m) => {
                        self.on.insert(key.clone(), m);
                    }
                    Err(e) => {
                        eprintln!("novaplayer: control {plugin}/{}: {e}", c.id);
                        continue;
                    }
                }
            }
            let m = &self.on[&key];
            for r in &c.reads {
                if let Some(v) = ctx.get(&plugin, r) {
                    doc.set_in(ui, m, r, v);
                }
            }
        }
    }
}
