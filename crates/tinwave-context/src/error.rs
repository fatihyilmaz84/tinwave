use thiserror::Error;

/// Errors of the context providers.
#[derive(Debug, Error)]
pub enum ContextError {
    /// The provider thread could not be started.
    #[error("could not start the {what} thread: {source}")]
    ThreadSpawn {
        /// Which thread.
        what: &'static str,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The provider thread has stopped, so the command could not be delivered.
    #[error("the audio provider thread is not running")]
    ProviderStopped,
    /// The provider thread is behind on commands; try again.
    #[error("the audio provider command queue is full")]
    CommandQueueFull,
}
