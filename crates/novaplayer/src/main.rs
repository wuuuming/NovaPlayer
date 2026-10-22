//! NovaPlayer's program: starts the app (`lib.rs`). Test runners are not product command-line modes.
#![cfg_attr(not(test), windows_subsystem = "windows")]

#[cfg(feature = "count-allocs")]
#[global_allocator]
static ALLOCATOR: novaui::trace::CountingAlloc = novaui::trace::CountingAlloc;

fn main() {
    novaplayer::run();
}
