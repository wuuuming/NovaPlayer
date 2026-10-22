//! The app's locks, handled in one place (principles B1).
//!
//! A lock is poisoned when a thread panicked while holding it. A release build aborts on a panic,
//! so poisoning only happens in debug builds and tests; there the data is taken as the panicking
//! thread left it, and the panic itself is what reports the bug.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// `Mutex::lock` that takes a poisoned lock's data as it is.
pub trait Locked<T: ?Sized> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `Condvar::wait` that takes a poisoned lock's data as it is.
pub fn wait<'a, T>(turn: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    turn.wait(guard).unwrap_or_else(PoisonError::into_inner)
}
