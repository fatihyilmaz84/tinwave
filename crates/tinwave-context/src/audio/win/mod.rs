//! The Windows backend: MMDevice notifications and endpoint-volume callbacks (ADR 0004).
//!
//! `com` holds every `unsafe` block of the crate. This module only wires it to the provider
//! thread.

mod com;
#[cfg(test)]
mod live_tests;

use std::sync::Arc;

use arc_swap::ArcSwap;

use super::runtime::{self, ProviderThread};
use super::stats::StatsCells;
use super::AudioMonitorConfig;
use crate::error::ContextError;
use crate::provider::ContextFields;

/// Starts the provider thread on the real audio stack. COM is initialised on that thread.
pub(super) fn spawn(
    config: AudioMonitorConfig,
    cache: Arc<ArcSwap<ContextFields>>,
    stats: Arc<StatsCells>,
) -> Result<ProviderThread, ContextError> {
    runtime::spawn(config, cache, stats, com::WindowsBackend::new)
}
