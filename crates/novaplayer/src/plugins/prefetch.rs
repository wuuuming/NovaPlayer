//! Thumbnail prefetch for the shown list.
use crate::app::*;
use novaui::Ui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

/// A song to fetch a thumbnail for: its place in the list, path, plugin and reference there.
type Item = (usize, PathBuf, String, String);

/// A list as the prefetch began on it: its songs, its play order, the plugins that offer
/// `thumbs`.
type Listed = (Arc<Vec<Arc<crate::library::Song>>>, Vec<usize>, HashSet<String>);

/// What is left of a list's prefetch, handed from one batch to the next (`Job::Thumbs`).
#[derive(Default)]
pub struct Rest {
    /// The list, still to be looked through (the first batch does it, off the UI thread).
    list: Option<Listed>,
    /// Songs still to fetch, in the list's order.
    todo: Vec<Item>,
    /// Where their small covers are, as the plugins said (asked a hundred at a time).
    urls: HashMap<String, String>,
}

impl Rest {
    fn is_empty(&self) -> bool {
        self.list.is_none() && self.todo.is_empty()
    }
}

/// Thumbnails fetched before a batch lands, so the rows show them as they come.
const BATCH: usize = 12;

impl Player {
    /// Prefetches the shown list's thumbnails while the network is otherwise idle: in the
    /// list's order, a plugin's batch call (`thumbs`) naming the covers, then one small picture
    /// at a time, a batch per task. Waits while a song opens or buffers (`turn` resumes it);
    /// the list moving on drops it (`Lists::moved`). A list's thumbnails, once kept, are not
    /// fetched again.
    #[cfg(all(feature = "test-support", windows))] // the screenshot suites (Windows only)
    pub fn prefetch_thumbs_now(&mut self, ui: &mut Ui) {
        self.plugin.prefetch = None;
        self.prefetch_thumbs(ui);
    }

    pub(crate) fn prefetch_thumbs(&mut self, ui: &mut Ui) {
        if self.plugin.prefetch == Some(self.lists.list_version) {
            if let Some(rest) = self.plugin.prefetch_waiting.take() {
                self.prefetch_more(ui, rest);
            }
            return;
        }
        self.plugin.prefetch = Some(self.lists.list_version);
        self.plugin.prefetch_waiting = None;
        // The plugin of each online song, if it offers `thumbs`.
        let able: HashSet<String> = self
            .plugin
            .registry
            .list
            .iter()
            .filter(|p| p.is_on() && p.manifest.as_ref().is_some_and(|m| m.provides.thumbs))
            .map(|p| p.id())
            .collect();
        if able.is_empty() {
            return;
        }
        // The list's songs are looked through on the task: a parse of every path of a
        // 20 000-song list took ~1.3 ms of the turn that landed it.
        let list = &self.lists.shown_list.list;
        let rest = Rest { list: Some((list.songs.clone(), list.order.clone(), able)), ..Default::default() };
        self.prefetch_more(ui, rest);
    }

    /// The next batch of the prefetch, unless the network is the player's now (then `turn`
    /// resumes it when it is free).
    pub(crate) fn prefetch_more(&mut self, ui: &mut Ui, rest: Rest) {
        if rest.is_empty() {
            return;
        }
        if self.plugin.net_busy.load(Relaxed) > 0 {
            self.plugin.prefetch_waiting = Some(rest);
            return;
        }
        let (busy, acc) = (self.plugin.net_busy.clone(), self.plugin.accounts.clone());
        let task = ui.spawn_waiting(
            move |cancel| {
                let mut rest = rest;
                if let Some((songs, order, able)) = rest.list.take() {
                    for &i in &order {
                        let Some(s) = songs.get(i) else { continue };
                        if let Some((plugin, uri)) = crate::plugins::client::parse(&s.path) {
                            if able.contains(&plugin) {
                                rest.todo.push((i, s.path.clone(), plugin, uri));
                            }
                        }
                    }
                }
                let mut idxs = Vec::new();
                let mut done = 0;
                while done < rest.todo.len() && idxs.len() < BATCH {
                    if cancel.cancelled() || busy.load(Relaxed) > 0 {
                        break;
                    }
                    if crate::plugins::client::kept_thumb(&rest.todo[done].1).is_some() {
                        done += 1;
                        continue;
                    }
                    if !rest.urls.contains_key(&rest.todo[done].3) {
                        // The next hundred not kept yet: where their covers are, by plugin.
                        let next: Vec<&Item> = rest.todo[done..]
                            .iter()
                            .filter(|t| crate::plugins::client::kept_thumb(&t.1).is_none())
                            .take(100)
                            .collect();
                        let mut by_plugin: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
                        for t in &next {
                            by_plugin.entry(t.2.as_str()).or_default().push(t.3.clone());
                        }
                        let mut urls = HashMap::new();
                        for (plugin, uris) in by_plugin {
                            urls.extend(crate::plugins::client::thumbs(&acc, plugin, &uris));
                        }
                        // A song the plugin named no cover for is not asked again.
                        for t in next {
                            urls.entry(t.3.clone()).or_default();
                        }
                        rest.urls.extend(urls);
                    }
                    let t = &rest.todo[done];
                    done += 1;
                    if let Some(u) = rest.urls.get(&t.3).filter(|u| !u.is_empty()) {
                        if crate::plugins::client::fetch_thumb(&t.1, u) {
                            idxs.push(t.0);
                        }
                    }
                }
                rest.todo.drain(..done);
                Job::Thumbs { idxs, rest }
            },
            Landed,
        );
        self.lists.tasks.hold(Some(task));
    }
}
