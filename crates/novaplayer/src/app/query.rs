//! What the user typed to find something, wherever they type it: the queue's search box, a
//! dropdown's box. One rule, so they never disagree: every word is found, in any case, in one
//! of the thing's names (a song's title, artist, album; a font's names).

pub struct Query(Vec<String>);

impl Query {
    pub fn new(text: &str) -> Self {
        Query(text.split_whitespace().map(str::to_lowercase).collect())
    }

    /// Nothing typed: everything matches.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn matches<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let names: Vec<String> = names.into_iter().map(str::to_lowercase).collect();
        self.0.iter().all(|w| names.iter().any(|n| n.contains(w.as_str())))
    }
}
