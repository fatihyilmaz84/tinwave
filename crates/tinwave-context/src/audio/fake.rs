//! Test doubles: a scriptable fake OS and a fake playback source. Test builds only.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tinwave_core::{DeviceId, RecordingId, SessionId, UtcMillis};
use tinwave_signals::{
    ElapsedDs, EventContext, PumpReport, Section, SignalDrain, SignalEvent, SignalSink,
    SignalsError,
};

use super::backend::{
    Backend, BindError, BindTarget, BoundEndpoint, DeviceDescription, DeviceState,
};
use super::classify::{form_factor, JackDescription, Transport};
use super::{PlaybackContextSource, PlaybackSnapshot};

/// One endpoint of the fake OS.
#[derive(Debug, Clone)]
pub(crate) struct FakeDevice {
    pub description: DeviceDescription,
    pub state: DeviceState,
    pub scalar: f32,
    pub muted: bool,
    pub gain_db: Option<f32>,
}

#[derive(Debug, Default)]
pub(crate) struct FakeOs {
    pub devices: BTreeMap<String, FakeDevice>,
    /// The default render endpoint (multimedia role).
    pub default_id: Option<String>,
    /// The endpoint the volume notification is registered on.
    pub bound_id: Option<String>,
    pub generation: u64,
    /// Calls the engine made, in order, for ordering assertions.
    pub calls: Vec<String>,
    /// Makes the next `bind` fail with this error text.
    pub fail_next_bind: Option<String>,
}

/// The test's handle on the fake OS; the backend holds the other end.
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeHandle(pub Arc<Mutex<FakeOs>>);

impl FakeHandle {
    pub(crate) fn os(&self) -> MutexGuard<'_, FakeOs> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn add(&self, id: &str, device: FakeDevice) {
        self.os().devices.insert(id.to_owned(), device);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.os().generation
    }

    pub(crate) fn backend(&self) -> FakeBackend {
        FakeBackend(self.clone())
    }
}

pub(crate) struct FakeBackend(FakeHandle);

impl std::fmt::Debug for FakeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeBackend")
    }
}

impl Backend for FakeBackend {
    fn bind(
        &mut self,
        target: BindTarget<'_>,
        generation: u64,
    ) -> Result<BoundEndpoint, BindError> {
        let mut os = self.0.os();
        os.calls.push(format!("bind {target:?}"));
        if let Some(text) = os.fail_next_bind.take() {
            return Err(BindError::Os(text));
        }
        assert!(
            os.bound_id.is_none(),
            "the engine must unbind before it binds"
        );
        let id = match target {
            BindTarget::SystemDefault => os.default_id.clone().ok_or(BindError::NoDevice)?,
            BindTarget::Device(id) => id.to_owned(),
        };
        let device = os.devices.get(&id).ok_or(BindError::NoDevice)?.clone();
        if device.state != DeviceState::Active {
            return Err(BindError::NotActive);
        }
        os.bound_id = Some(id.clone());
        os.generation = generation;
        Ok(BoundEndpoint {
            id,
            description: device.description,
            volume_scalar: device.scalar,
            muted: device.muted,
        })
    }

    fn unbind(&mut self) {
        let mut os = self.0.os();
        os.calls.push("unbind".to_owned());
        os.bound_id = None;
    }

    fn gain_db(&mut self) -> Option<f32> {
        let os = self.0.os();
        let id = os.bound_id.as_ref()?;
        os.devices.get(id)?.gain_db
    }
}

/// A Realtek-like desktop line-out.
pub(crate) fn speakers() -> FakeDevice {
    FakeDevice {
        description: DeviceDescription {
            form_factor: Some(form_factor::SPEAKERS),
            transport: Transport::HdAudio,
            jack: Some(JackDescription {
                connection_type: 1,
                geo_location: 1,
                gen_location: 0,
            }),
        },
        state: DeviceState::Active,
        scalar: 0.30,
        muted: false,
        gain_db: Some(-40.1),
    }
}

/// A Bluetooth A2DP headset.
pub(crate) fn bluetooth_headset() -> FakeDevice {
    FakeDevice {
        description: DeviceDescription {
            form_factor: Some(form_factor::HEADPHONES),
            transport: Transport::Bluetooth,
            jack: None,
        },
        state: DeviceState::Active,
        scalar: 0.50,
        muted: false,
        gain_db: Some(-20.0),
    }
}

/// A monitor over HDMI/DisplayPort.
pub(crate) fn monitor() -> FakeDevice {
    FakeDevice {
        description: DeviceDescription {
            form_factor: Some(form_factor::DIGITAL_AUDIO_DISPLAY_DEVICE),
            transport: Transport::HdAudio,
            jack: None,
        },
        state: DeviceState::Active,
        scalar: 0.80,
        muted: false,
        gain_db: None,
    }
}

/// The playback source of the tests: a switch plus fixed event columns.
#[derive(Debug, Default)]
pub(crate) struct FakePlayback {
    playing: Mutex<bool>,
}

impl FakePlayback {
    pub(crate) fn playing() -> Arc<Self> {
        Arc::new(Self {
            playing: Mutex::new(true),
        })
    }

    pub(crate) fn set_playing(&self, on: bool) {
        *self
            .playing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = on;
    }

    pub(crate) fn recording() -> RecordingId {
        RecordingId::from_parts(UtcMillis::new(T0), [3; 10]).expect("fixed test id")
    }

    pub(crate) fn session() -> SessionId {
        SessionId::from_parts(UtcMillis::new(T0), [2; 10]).expect("fixed test id")
    }
}

impl PlaybackContextSource for FakePlayback {
    fn current(&self) -> Option<PlaybackSnapshot> {
        let on = *self
            .playing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        on.then(|| PlaybackSnapshot {
            event: EventContext {
                session: Self::session(),
                recording: Some(Self::recording()),
                source: None,
                pos_ds: Some(1_234),
                ctx: None,
            },
            section: Section::UNKNOWN,
            since_track_start: ElapsedDs::from_ds(100),
            since_section_start: ElapsedDs::from_ds(10),
        })
    }
}

/// 2026-11-02T00:00:00Z.
pub(crate) const T0: i64 = 1_793_577_600_000;

pub(crate) fn device_id() -> DeviceId {
    DeviceId::from_parts(UtcMillis::new(T0), [1; 10]).expect("fixed test id")
}

/// A sink that keeps everything in memory.
#[derive(Debug, Default)]
pub(crate) struct VecSink {
    pub events: Vec<SignalEvent>,
}

impl SignalSink for VecSink {
    fn write(
        &mut self,
        events: &[SignalEvent],
        _counters: &[tinwave_signals::CounterRow],
    ) -> Result<(), SignalsError> {
        self.events.extend_from_slice(events);
        Ok(())
    }
}

impl VecSink {
    /// Pumps until the drain has nothing more and returns every event seen so far.
    pub(crate) fn pump_all(&mut self, drain: &mut SignalDrain) -> &[SignalEvent] {
        loop {
            let report: PumpReport = drain.pump(self).expect("the in-memory sink cannot fail");
            if !report.more {
                break;
            }
        }
        &self.events
    }
}
