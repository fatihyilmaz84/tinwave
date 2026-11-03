//! Shared fixtures for the unit tests of the bus, the coalescer and the counters.

use std::sync::Arc;

use tinwave_core::{DeviceId, ManualClock, RecordingId, SessionId, UtcMillis};

use crate::bus::{BusConfig, EventStamp, SignalBus, SignalDrain, SignalSink};
use crate::coalesce::EventContext;
use crate::counters::CounterRow;
use crate::derived::DerivedValue;
use crate::{SignalEvent, SignalPayload, SignalSettings, SignalsError};

/// 2026-11-02T00:00:00Z.
pub(crate) const T0: i64 = 1_793_577_600_000;

pub(crate) fn device() -> DeviceId {
    DeviceId::from_parts(UtcMillis::new(T0), [1; 10]).unwrap()
}

pub(crate) fn session() -> SessionId {
    SessionId::from_parts(UtcMillis::new(T0), [2; 10]).unwrap()
}

pub(crate) fn recording(n: u8) -> RecordingId {
    RecordingId::from_parts(UtcMillis::new(T0), [n; 10]).unwrap()
}

pub(crate) fn context() -> EventContext {
    EventContext {
        session: session(),
        recording: Some(recording(3)),
        source: None,
        pos_ds: None,
        ctx: None,
    }
}

pub(crate) fn event(stamp: EventStamp, payload: SignalPayload) -> SignalEvent {
    SignalEvent {
        id: stamp.id,
        ts: stamp.ts,
        session: session(),
        recording: Some(recording(3)),
        source: None,
        pos_ds: None,
        payload,
        ctx: None,
    }
}

/// A sink that keeps everything in memory and can be told to fail once.
#[derive(Default)]
pub(crate) struct VecSink {
    pub(crate) events: Vec<SignalEvent>,
    pub(crate) rows: Vec<CounterRow>,
    pub(crate) derived: Vec<DerivedValue>,
    pub(crate) fail_next: bool,
    pub(crate) calls: usize,
}

impl SignalSink for VecSink {
    fn write(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
    ) -> Result<(), SignalsError> {
        self.calls += 1;
        if std::mem::take(&mut self.fail_next) {
            return Err(SignalsError::Sink {
                reason: "disk full".into(),
            });
        }
        self.events.extend_from_slice(events);
        self.rows.extend_from_slice(counters);
        Ok(())
    }

    fn write_with_derived(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
        derived: &[DerivedValue],
    ) -> Result<(), SignalsError> {
        self.write(events, counters)?;
        self.derived.extend_from_slice(derived);
        Ok(())
    }
}

impl VecSink {
    /// Pumps until the drain reports nothing more, returning every event written so far.
    pub(crate) fn pump_all(&mut self, drain: &mut SignalDrain) -> &[SignalEvent] {
        while drain.pump(&mut *self).unwrap().more {}
        &self.events
    }
}

pub(crate) fn bus_with(
    config: impl FnOnce(&mut BusConfig),
) -> (SignalBus, SignalDrain, ManualClock) {
    let clock = ManualClock::new(UtcMillis::new(T0));
    let mut cfg = BusConfig::new(device());
    config(&mut cfg);
    let (bus, drain) =
        SignalBus::channel(cfg, clock.clone(), Arc::new(SignalSettings::new())).unwrap();
    (bus, drain, clock)
}
