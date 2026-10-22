//! The vault: the credentials a plugin's sign-in returned, opaque to the player, kept in the system's store under
//! `NovaPlayer/<plugin id>`: Windows Credential Manager, or the Secret Service on Linux (over
//! D-Bus). A credential is handed only to the plugin it belongs to.
//!
//! The account name shown in the app menu is kept beside the credential (not secret).

use crate::sync::Locked;
use std::collections::HashMap;
use std::sync::Mutex;

/// What is known of each plugin's account this run: its credential and display name, read from
/// the store once. `None` inside: asked, not signed in; an entry not there: not read yet.
#[derive(Default)]
struct Known {
    credential: Option<Option<String>>,
    name: Option<Option<String>>,
}

/// The accounts, one per plugin. One cache for credentials and names, so a sign-out cannot leave
/// a name behind its credential. Owned by `Player` (`Plugins::accounts`); the tasks that call
/// plugins get a shared handle.
///
/// The system's store may wait on the user (a locked keyring's password prompt), so it is read
/// and written only off the UI thread: the views read what is known (`name`), and the store is
/// read at the start (`read`, a task) and written by a sign-in's or a sign-out's task.
#[derive(Default)]
pub struct Accounts {
    known: Mutex<HashMap<String, Known>>,
}

fn target(plugin: &str) -> String {
    format!("NovaPlayer/{plugin}")
}

impl Accounts {
    /// The plugin's credential, if its account is signed in: read from the store the first
    /// time (off the UI thread: a plugin call's task).
    pub fn credential(&self, plugin: &str) -> Option<String> {
        if let Some(c) = self.known.locked().get(plugin).and_then(|k| k.credential.clone()) {
            return c;
        }
        let read = store::read(&target(plugin));
        self.known.locked().entry(plugin.to_owned()).or_default().credential.get_or_insert(read).clone()
    }

    /// Reads the accounts of `plugins` not read yet, credential and name (a task's: the store
    /// may wait on the user).
    pub fn read(&self, plugins: &[String]) {
        for p in plugins {
            if self.credential(p).is_none() {
                continue;
            }
            if self.known.locked().get(p).is_some_and(|k| k.name.is_some()) {
                continue;
            }
            let name = store::user(&target(p));
            self.known.locked().entry(p.clone()).or_default().name.get_or_insert(name);
        }
    }

    /// Keeps `credential` for `plugin` (and the account's display name). A sign-in's task.
    pub fn save(&self, plugin: &str, credential: &str, account: &str) -> Result<(), String> {
        store::write(&target(plugin), account, credential)?;
        let known = Known { credential: Some(Some(credential.to_owned())), name: Some(Some(account.to_owned())) };
        self.known.locked().insert(plugin.to_owned(), known);
        Ok(())
    }

    /// The plugin's account is gone for everyone at once (a sign-out); `erase` takes it out of
    /// the store after. The credential as known, `None` when it was never read.
    pub fn forget(&self, plugin: &str) -> Option<Option<String>> {
        let gone = Known { credential: Some(None), name: Some(None) };
        self.known.locked().insert(plugin.to_owned(), gone).and_then(|k| k.credential)
    }

    /// Takes `plugin`'s account out of the store (a sign-out's task): the credential it held,
    /// for the plugin's `connect.end` (`known`: as `forget` gave it).
    pub fn erase(plugin: &str, known: Option<Option<String>>) -> Option<String> {
        let target = target(plugin);
        let credential = known.unwrap_or_else(|| store::read(&target));
        store::delete(&target);
        credential
    }

    /// The signed-in account's name, for display, as read so far: never the store.
    pub fn name(&self, plugin: &str) -> Option<String> {
        let k = self.known.locked();
        let k = k.get(plugin)?;
        k.credential.as_ref()?.as_ref()?;
        k.name.clone().flatten()
    }
}

/// The system's store. Tests never reach it: under `cfg(test)` or `test-support`, `store` is an in-memory map.
#[cfg(windows)]
#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
mod os_store {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Security::Credentials::*;

    pub fn read(target: &str) -> Option<String> {
        // SAFETY: on success CredReadW points `p` at a credential it allocated, read here and
        // freed once; its blob holds `CredentialBlobSize` bytes (an empty one may be null, so it
        // is not read). The target is a live HSTRING for the call.
        unsafe {
            let mut p: *mut CREDENTIALW = std::ptr::null_mut();
            CredReadW(&HSTRING::from(target), CRED_TYPE_GENERIC, 0, &mut p).ok()?;
            let c = &*p;
            let b = match c.CredentialBlobSize as usize {
                0 => &[][..],
                n => std::slice::from_raw_parts(c.CredentialBlob, n),
            };
            let t = String::from_utf8(b.to_vec()).ok();
            CredFree(p as *const _);
            t
        }
    }

    pub fn user(target: &str) -> Option<String> {
        // SAFETY: as in `read`; the user name is a NUL-terminated string inside the credential,
        // copied before it is freed.
        unsafe {
            let mut p: *mut CREDENTIALW = std::ptr::null_mut();
            CredReadW(&HSTRING::from(target), CRED_TYPE_GENERIC, 0, &mut p).ok()?;
            let u = (*p).UserName.to_string().ok();
            CredFree(p as *const _);
            u.filter(|u| !u.is_empty())
        }
    }

    pub fn write(target: &str, user: &str, credential: &str) -> Result<(), String> {
        let mut t: Vec<u16> = target.encode_utf16().chain([0]).collect();
        let mut u: Vec<u16> = user.encode_utf16().chain([0]).collect();
        let mut blob = credential.as_bytes().to_vec();
        let c = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(t.as_mut_ptr()),
            UserName: PWSTR(u.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: `t` and `u` are NUL-terminated and `blob` holds `CredentialBlobSize` bytes; all
        // outlive the call, which copies them.
        unsafe { CredWriteW(&c, 0) }.map_err(|e| novaui::i18n::t("err.sign-in-save", &[("e", &e.to_string())]))
    }

    pub fn delete(target: &str) {
        // SAFETY: the target is a live HSTRING for the call.
        unsafe {
            let _ = CredDeleteW(&HSTRING::from(target), CRED_TYPE_GENERIC, 0);
        }
    }
}

/// The freedesktop Secret Service, spoken to over D-Bus (KWallet, GNOME Keyring, KeePassXC all
/// offer it): no helper program needed. Items carry the attributes `service` and `kind`, as
/// `secret-tool` wrote them before.
#[cfg(not(windows))]
#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
mod os_store {
    use secret_service::blocking::SecretService;
    use secret_service::EncryptionType;
    use std::collections::HashMap;

    fn attrs<'a>(target: &'a str, kind: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([("service", target), ("kind", kind)])
    }

    fn connect() -> Result<SecretService<'static>, String> {
        SecretService::connect(EncryptionType::Dh)
            .map_err(|e| novaui::i18n::t("err.no-secret-service", &[("e", &e.to_string())]))
    }

    fn lookup(target: &str, kind: &str) -> Option<String> {
        let ss = connect().ok()?;
        let found = ss.search_items(attrs(target, kind)).ok()?;
        let item = match found.unlocked.into_iter().next() {
            Some(i) => i,
            None => {
                let i = found.locked.into_iter().next()?;
                i.unlock().ok()?;
                i
            }
        };
        String::from_utf8(item.get_secret().ok()?).ok().filter(|s| !s.is_empty())
    }

    pub fn read(target: &str) -> Option<String> {
        lookup(target, "token")
    }

    pub fn user(target: &str) -> Option<String> {
        lookup(target, "account")
    }

    fn store(target: &str, kind: &str, value: &str) -> Result<(), String> {
        let ss = connect()?;
        // A desktop whose keyring was never set up has no collection at all (KWallet before its
        // first wallet): one is created, and the keyring asks the user for its password once.
        let c = ss
            .get_default_collection()
            .or_else(|_| ss.get_any_collection())
            .or_else(|_| ss.create_collection("Default", "default"))
            .map_err(|e| novaui::i18n::t("err.no-keyring", &[("e", &e.to_string())]))?;
        c.ensure_unlocked().map_err(|e| novaui::i18n::t("err.keyring-locked", &[("e", &e.to_string())]))?;
        c.create_item(&format!("{target} ({kind})"), attrs(target, kind), value.as_bytes(), true, "text/plain")
            .map(|_| ())
            .map_err(|e| novaui::i18n::t("err.sign-in-save", &[("e", &e.to_string())]))
    }

    pub fn write(target: &str, user: &str, credential: &str) -> Result<(), String> {
        // The keyring may put up its own prompt (a first wallet's password, an unlock) and wait on
        // the user: the sign-in is not held past two minutes for it.
        let (t, u, k) = (target.to_owned(), user.to_owned(), credential.to_owned());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(store(&t, "token", &k).and_then(|_| store(&t, "account", &u)));
        });
        rx.recv_timeout(std::time::Duration::from_secs(120))
            .unwrap_or_else(|_| Err(novaui::i18n::t("err.keyring-silent", &[])))
    }

    pub fn delete(target: &str) {
        let Ok(ss) = connect() else { return };
        for kind in ["token", "account"] {
            if let Ok(found) = ss.search_items(attrs(target, kind)) {
                for i in found.unlocked.into_iter().chain(found.locked) {
                    let _ = i.delete();
                }
            }
        }
    }

    /// Saves, reads and forgets a throwaway entry (run on a Linux desktop by hand).
    #[cfg(test)]
    #[test]
    #[ignore = "L5 manual: a Linux desktop's secret service"]
    fn secret_service_round_trip() {
        let t = "NovaPlayer/test-probe";
        write(t, "测试账号", "credential-123").expect("saved");
        assert_eq!(read(t).as_deref(), Some("credential-123"));
        assert_eq!(user(t).as_deref(), Some("测试账号"));
        delete(t);
        assert_eq!(read(t), None);
    }
}

#[cfg(not(any(test, feature = "test-support")))]
use os_store as store;

/// Every test, the screenshot harness included, gets a vault of its own process: no account
/// left by one run is seen by the next, and no test writes the user's keyring.
#[cfg(any(test, feature = "test-support"))]
mod store {
    use crate::sync::Locked;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// target → (account, credential)
    static VAULT: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None);

    fn with<T>(f: impl FnOnce(&mut HashMap<String, (String, String)>) -> T) -> T {
        f(VAULT.locked().get_or_insert_with(HashMap::new))
    }

    pub fn read(target: &str) -> Option<String> {
        with(|v| v.get(target).map(|(_, c)| c.clone()))
    }

    pub fn user(target: &str) -> Option<String> {
        with(|v| v.get(target).map(|(u, _)| u.clone()))
    }

    pub fn write(target: &str, user: &str, credential: &str) -> Result<(), String> {
        with(|v| v.insert(target.to_owned(), (user.to_owned(), credential.to_owned())));
        Ok(())
    }

    pub fn delete(target: &str) {
        with(|v| v.remove(target));
    }
}
