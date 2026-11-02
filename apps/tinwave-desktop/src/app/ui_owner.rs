//! The UI owner: the QML engine together with every object whose lifetime is tied to it (image providers).
//!
//! Qt destroys an engine's image providers and Qt Quick's pixmap-reader thread inside the engine's destructor. Worker
//! threads still running at that moment crashed `~QThread` in about 1 percent of runs (ADR 0008, T-0041 crash B), and
//! relying on a provider's destructor alone hung at exit in 2 of 300. So the rule is structural, not a convention:
//! **every provider is quiesced before the engine is dropped**, and the only way to hold both is this type.
//!
//! `main` calls [`UiOwner::shutdown`] explicitly after `exec()` returns; [`Drop`] repeats it (idempotent) and then
//! drops the engine, so an early return or a panic cannot skip the order.

use std::fmt;
use std::rc::Rc;

use super::artwork_bridge::ArtworkProvider;

/// Something with worker threads that must stop before the engine goes away.
pub trait Quiesce {
    /// Stops every worker thread of this object and waits for them. Idempotent. GUI thread only.
    fn quiesce(&self);
}

impl Quiesce for ArtworkProvider {
    fn quiesce(&self) {
        ArtworkProvider::quiesce(self);
    }
}

/// Owns the engine `E` (a `QQmlApplicationEngine` in the app) and the providers that depend on it.
pub struct UiOwner<E> {
    // Field order matters: `Drop::drop` (the quiesce) runs first, then `engine` is dropped, and only then the provider
    // handles, which must outlive the engine because the engine's copies of the providers share their state.
    engine: E,
    providers: Vec<Rc<dyn Quiesce>>,
}

impl<E> fmt::Debug for UiOwner<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UiOwner")
            .field("providers", &self.providers.len())
            .finish()
    }
}

impl<E> UiOwner<E> {
    /// Takes ownership of `engine`.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            providers: Vec::new(),
        }
    }

    /// Registers a provider that was installed on the engine. It is quiesced before the engine is dropped.
    pub fn add_provider(&mut self, provider: Rc<dyn Quiesce>) {
        self.providers.push(provider);
    }

    /// The engine, for loading QML and installing providers.
    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    /// Quiesces every provider, in registration order. Call it after the event loop ended. Idempotent.
    pub fn shutdown(&mut self) {
        for provider in &self.providers {
            provider.quiesce();
        }
    }
}

impl<E> Drop for UiOwner<E> {
    fn drop(&mut self) {
        // The quiesce comes first; the fields (engine, then provider handles) are dropped after this body returns.
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    type Log = Rc<RefCell<Vec<String>>>;

    struct FakeEngine(Log);
    impl Drop for FakeEngine {
        fn drop(&mut self) {
            self.0.borrow_mut().push("engine dropped".to_owned());
        }
    }

    struct FakeProvider(&'static str, Log);
    impl Quiesce for FakeProvider {
        fn quiesce(&self) {
            self.1.borrow_mut().push(format!("quiesce {}", self.0));
        }
    }

    fn owner_with_two_providers(log: &Log) -> UiOwner<FakeEngine> {
        let mut owner = UiOwner::new(FakeEngine(Rc::clone(log)));
        owner.add_provider(Rc::new(FakeProvider("art", Rc::clone(log))));
        owner.add_provider(Rc::new(FakeProvider("cover", Rc::clone(log))));
        owner
    }

    #[test]
    fn drop_quiesces_every_provider_before_the_engine() {
        let log = Log::default();
        drop(owner_with_two_providers(&log));
        assert_eq!(
            *log.borrow(),
            ["quiesce art", "quiesce cover", "engine dropped"]
        );
    }

    #[test]
    fn explicit_shutdown_does_not_drop_the_engine_and_drop_repeats_it_before_the_engine() {
        let log = Log::default();
        let mut owner = owner_with_two_providers(&log);
        owner.shutdown();
        assert_eq!(*log.borrow(), ["quiesce art", "quiesce cover"]);
        drop(owner);
        assert_eq!(
            *log.borrow(),
            [
                "quiesce art",
                "quiesce cover",
                "quiesce art",
                "quiesce cover",
                "engine dropped"
            ],
            "Drop quiesces again (idempotent) and still drops the engine last"
        );
    }

    #[test]
    fn an_owner_without_providers_just_drops_the_engine() {
        let log = Log::default();
        drop(UiOwner::new(FakeEngine(Rc::clone(&log))));
        assert_eq!(*log.borrow(), ["engine dropped"]);
    }

    #[test]
    fn the_engine_stays_reachable_until_the_owner_is_dropped() {
        let log = Log::default();
        let mut owner = UiOwner::new(FakeEngine(Rc::clone(&log)));
        let _ = owner.engine_mut();
        assert!(log.borrow().is_empty());
    }
}
