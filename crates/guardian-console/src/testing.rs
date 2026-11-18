//! `FakeBackend` for UI tests (feature `testing`): records actions, returns
//! the events a test pushes.

use std::sync::{Arc, Mutex};

use crate::backend::{Action, Backend, Event};

#[derive(Default)]
struct Inner {
    sent: Mutex<Vec<Action>>,
    queued: Mutex<Vec<Event>>,
}

/// A backend the test drives by hand. Clones share the same queues.
#[derive(Clone, Default)]
pub struct FakeBackend(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

impl FakeBackend {
    /// The UI gets `e` at its next frame.
    pub fn push(&self, e: Event) {
        lock(&self.0.queued).push(e);
    }

    /// Everything the UI sent so far.
    pub fn sent(&self) -> Vec<Action> {
        lock(&self.0.sent).clone()
    }

    /// Forgets the sent actions.
    pub fn clear(&self) {
        lock(&self.0.sent).clear();
    }
}

impl Backend for FakeBackend {
    fn send(&self, a: Action) {
        lock(&self.0.sent).push(a);
    }

    fn poll(&self) -> Vec<Event> {
        std::mem::take(&mut *lock(&self.0.queued))
    }
}
