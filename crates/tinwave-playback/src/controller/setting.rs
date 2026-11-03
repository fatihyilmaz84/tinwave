//! The controller's view of the listener's exclusive-mode setting.
//!
//! The setting lives in the output (`AudioOutput::wants_exclusive`: `WasapiControl::set_exclusive`,
//! `ManualHandle::set_exclusive`), which the `Engine` owns. The controller hands the engine a
//! [`SharedOutput`] and keeps an [`ExclusiveSetting`] on the same output, so it can read the setting
//! whenever it drains, without the engine's help and whatever the stream is doing.
//!
//! Every call into the output happens on the thread that drives the controller (the engine opens
//! and reopens streams from `poll` and the commands, never from its decoder thread), so the lock
//! is never contended; it only makes the two views of one output safe to hold.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::{AudioOutput, OpenRequest, OutputStream, PlaybackError, Renderer};

type Shared = Arc<Mutex<Box<dyn AudioOutput>>>;

fn lock(output: &Shared) -> MutexGuard<'_, Box<dyn AudioOutput>> {
    output.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The output as the engine sees it. It forwards **every** `AudioOutput` method: a method added to
/// the trait with a default must be forwarded here too, or the engine would silently get the
/// default instead of the real output's answer.
struct SharedOutput(Shared);

impl AudioOutput for SharedOutput {
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        lock(&self.0).open(request, renderer)
    }

    fn wants_exclusive(&self) -> bool {
        lock(&self.0).wants_exclusive()
    }
}

/// Reads the listener's exclusive-mode setting from the output the engine is playing through.
pub(super) struct ExclusiveSetting(Shared);

impl ExclusiveSetting {
    /// Splits `output` into the one for the engine and the reader of its setting.
    pub(super) fn split(output: Box<dyn AudioOutput>) -> (Box<dyn AudioOutput>, Self) {
        let shared: Shared = Arc::new(Mutex::new(output));
        (Box::new(SharedOutput(Arc::clone(&shared))), Self(shared))
    }

    /// Whether the listener wants exclusive mode, as of now.
    pub(super) fn wanted(&self) -> bool {
        lock(&self.0).wants_exclusive()
    }
}
