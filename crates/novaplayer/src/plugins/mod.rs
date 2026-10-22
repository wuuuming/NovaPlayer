pub mod accounts;
pub mod client;
pub mod connect;
pub mod controls;
pub mod host;
pub mod pcm;
pub mod prefetch;
pub mod prints;
pub mod registry;

/// A plugin's sign-in form: the values its fields hold, by index (kept across rebuilds; cleared
/// when another plugin's form shows or a connection succeeded).
#[derive(Default)]
pub struct ConnectForm {
    pub plugin: String,
    pub values: Vec<String>,
}

impl Plugins {
    /// A sign-in or a sign-out: what was asked of the accounts as they were is dropped.
    pub fn accounts_moved(&mut self) {
        self.account_version += 1;
        self.account_tasks.clear();
    }

    /// The connection in progress, if any, is taken and called off (the plugin is told); its
    /// plugin.
    pub fn call_off_connection(&mut self, ui: &mut novaui::Ui) -> Option<String> {
        let c = self.connection.take()?;
        let plugin = c.plugin.clone();
        crate::plugins::connect::call_off(&mut self.errands, ui, c);
        Some(plugin)
    }

    /// A connection in progress for `id` is called off (the plugin is told).
    pub fn drop_connection(&mut self, ui: &mut novaui::Ui, id: &str) {
        if self.connection.as_ref().is_some_and(|s| s.plugin == id) {
            self.call_off_connection(ui);
        }
    }

    /// `id` signs out: no connection in progress, account answers asked before are dropped, the
    /// plugin is told (`connect.end`) with the credential it gave, and the vault forgets it.
    pub fn sign_out(&mut self, ui: &mut novaui::Ui, id: &str) {
        self.drop_connection(ui, id);
        self.accounts_moved();
        // Gone at once for the views; the store and the plugin are told off the UI thread.
        let known = self.accounts.forget(id);
        let plugin = id.to_owned();
        let task = ui.spawn_waiting(
            move |_| {
                if let Some(c) = crate::plugins::accounts::Accounts::erase(&plugin, known) {
                    crate::plugins::connect::end_now(&plugin, &c);
                }
            },
            |()| (),
        );
        self.errands.hold(Some(task));
    }
}

impl ConnectForm {
    /// What field `k` of `plugin`'s form holds (empty for another plugin's form).
    pub fn kept(&self, plugin: &str, k: usize) -> &str {
        if self.plugin != plugin {
            ""
        } else {
            self.values.get(k).map_or("", String::as_str)
        }
    }

    /// The field `k` of `plugin`'s form was typed in.
    pub fn set(&mut self, plugin: &str, k: usize, text: String) {
        if self.plugin != plugin {
            self.plugin = plugin.to_owned();
            self.values.clear();
        }
        if self.values.len() <= k {
            self.values.resize(k + 1, String::new());
        }
        self.values[k] = text;
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }
}

/// `Player` state owned by this area.
pub struct Plugins {
    /// Installed plugins and which are enabled (`plugins.rs`).
    pub registry: crate::plugins::registry::Registry,
    /// The controls plugins add, mounted in their slots (controls.rs).
    pub plugin_controls: crate::plugins::controls::Mounts,
    /// The list (its version) the thumbnail prefetch is for; its batches are the list's tasks.
    pub prefetch: Option<u64>,
    /// The prefetch's next batch, waiting for the network to be free.
    pub prefetch_waiting: Option<crate::plugins::prefetch::Rest>,
    /// Requests made for the accounts as they are (a signed-in account's lists): a sign-in or
    /// sign-out drops them (`accounts_moved`).
    pub account_tasks: crate::app::Tasks,
    /// What must be done whatever the app shows next: telling a plugin a session or an account
    /// ended, forgetting an account in the system's store, clearing a cache.
    pub errands: crate::app::Tasks,
    /// The network is busy for the player's own needs (a song opening, buffering, a face being
    /// asked for): the prefetch waits.
    pub net_busy: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// A connection in progress (`connect.rs`).
    pub connection: Option<crate::plugins::connect::Connection>,
    /// What the user typed into a plugin's sign-in form (its flyout's or its settings page's:
    /// one form, wherever it shows).
    pub form: ConnectForm,
    /// Bumped by every connect and disconnect: account answers asked before are dropped.
    pub account_version: u64,
    /// The plugins' accounts (credentials and names); shared with the workers that call plugins.
    pub accounts: std::sync::Arc<crate::plugins::accounts::Accounts>,
}
