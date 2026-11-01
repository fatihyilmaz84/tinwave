//! T-0005 acceptance tests, through the public API only.

// Test code: a failed unwrap is the failure report. clippy.toml only exempts `#[test]` bodies, not
// helpers in integration-test crates.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::sync::Arc;

use proptest::prelude::*;
use tinwave_core::{DeviceId, ManualClock, RecordingId, SessionId, Tier, UtcMillis};
use tinwave_signals::{
    BusConfig, CounterEntity, CounterRow, ElapsedDs, EmitOutcome, EventContext, EventStamp,
    GestureCoalescer, GestureTick, GestureTime, InputSource, PausedPayload, Priority, Section,
    SignalBus, SignalCategory, SignalDrain, SignalEvent, SignalKind, SignalPayload, SignalSettings,
    SignalSink, SignalsError, TickOutcome, View, ViewOpenedPayload, Volume, VolumePayload,
};

const T0: i64 = 1_793_577_600_000; // 2026-11-02T00:00:00Z

fn at() -> UtcMillis {
    UtcMillis::new(T0)
}

#[derive(Default)]
struct MemorySink {
    events: Vec<SignalEvent>,
    rows: Vec<CounterRow>,
}

impl SignalSink for MemorySink {
    fn write(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
    ) -> Result<(), SignalsError> {
        self.events.extend_from_slice(events);
        self.rows.extend_from_slice(counters);
        Ok(())
    }
}

fn setup(config: impl FnOnce(&mut BusConfig)) -> (SignalBus, SignalDrain, ManualClock) {
    let clock = ManualClock::new(at());
    let mut cfg = BusConfig::new(DeviceId::from_parts(at(), [1; 10]).unwrap());
    config(&mut cfg);
    let (bus, drain) =
        SignalBus::channel(cfg, clock.clone(), Arc::new(SignalSettings::new())).unwrap();
    (bus, drain, clock)
}

fn event(stamp: EventStamp, payload: SignalPayload) -> SignalEvent {
    SignalEvent {
        id: stamp.id,
        ts: stamp.ts,
        session: SessionId::from_parts(at(), [2; 10]).unwrap(),
        recording: Some(RecordingId::from_parts(at(), [3; 10]).unwrap()),
        source: None,
        pos_ds: None,
        payload,
        ctx: None,
    }
}

fn paused() -> SignalPayload {
    SignalPayload::Paused(PausedPayload {
        input: InputSource::MediaKey,
    })
}

fn view_opened() -> SignalPayload {
    SignalPayload::ViewOpened(ViewOpenedPayload {
        view: View::NowPlaying,
    })
}

fn drain_all(drain: &mut SignalDrain) -> MemorySink {
    let mut sink = MemorySink::default();
    while drain.pump(&mut sink).unwrap().more {}
    sink
}

#[test]
fn overflow_flood_of_low_priority_events_keeps_every_playback_event_and_counts_drops() {
    let (bus, mut drain, _) = setup(|c| {
        c.low_capacity = 128;
        c.reserved_capacity = 2_048;
    });
    const FLOOD: u64 = 100_000;
    const PLAYBACK: u64 = 1_500;
    let mut expected_reserved = Vec::new();
    for i in 0..FLOOD {
        let outcome = bus
            .emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()))
            .unwrap();
        assert_ne!(outcome, EmitOutcome::Disabled);
        if i % (FLOOD / PLAYBACK) == 0 && (expected_reserved.len() as u64) < PLAYBACK {
            let seq = Cell::new(0);
            let out = bus
                .emit_with(SignalKind::Paused, |s| {
                    seq.set(s.id.seq);
                    event(s, paused())
                })
                .unwrap();
            assert_eq!(
                out,
                EmitOutcome::Queued,
                "playback events are never refused"
            );
            expected_reserved.push(seq.get());
        }
    }
    let sink = drain_all(&mut drain);

    let delivered_reserved: Vec<u64> = sink
        .events
        .iter()
        .filter(|e| Priority::of(e.kind()) == Priority::Reserved)
        .map(|e| e.id.seq)
        .collect();
    assert_eq!(
        delivered_reserved, expected_reserved,
        "all T0 events survive, in order"
    );
    assert_eq!(bus.reserved_overflows(), 0);
    assert_eq!(bus.dropped(SignalKind::Paused), 0);

    let delivered_low = sink
        .events
        .iter()
        .filter(|e| e.kind() == SignalKind::ViewOpened)
        .count() as u64;
    assert_eq!(
        delivered_low, 128,
        "the queue ended full: only the newest 128 survive"
    );
    assert_eq!(
        bus.dropped(SignalKind::ViewOpened),
        FLOOD - delivered_low,
        "drop counter is exact"
    );
    assert_eq!(bus.dropped_total(), FLOOD - delivered_low);
    assert_eq!(
        bus.dropped_by_kind(),
        [(SignalKind::ViewOpened, FLOOD - 128)]
    );
}

#[test]
fn the_newest_low_priority_events_are_the_ones_kept() {
    let (bus, mut drain, _) = setup(|c| c.low_capacity = 10);
    for _ in 0..25 {
        let _ = bus.emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()));
    }
    let seqs: Vec<_> = drain_all(&mut drain)
        .events
        .iter()
        .map(|e| e.id.seq)
        .collect();
    assert_eq!(seqs, (15..25).collect::<Vec<_>>());
}

#[test]
fn a_disabled_category_constructs_zero_events() {
    // Every event kind, with its category switched off: the counting constructor must never run.
    for category in SignalCategory::ALL {
        let (bus, mut drain, _) = setup(|_| {});
        bus.settings().set_category(category, false);
        let built = Cell::new(0_u32);
        for kind in SignalKind::events().filter(|&k| SignalCategory::of(k) == category) {
            for _ in 0..100 {
                let outcome = bus.emit_with(kind, |_| {
                    built.set(built.get() + 1);
                    unreachable!("constructor ran for disabled {kind}")
                });
                assert_eq!(outcome, Ok(EmitOutcome::Disabled), "{kind}");
            }
        }
        assert_eq!(built.get(), 0, "{category:?}");
        assert_eq!(drain_all(&mut drain).events.len(), 0);
        assert_eq!(bus.queued_total(), 0);
    }
}

#[test]
fn a_disabled_tier_constructs_zero_events_and_enabled_kinds_still_flow() {
    let (bus, mut drain, _) = setup(|_| {});
    bus.settings().set_tier(Tier::T1, false);
    let built = Cell::new(0_u32);
    let out = bus.emit_with(SignalKind::OutputDeviceChanged, |_| {
        built.set(built.get() + 1);
        unreachable!("T1 is off")
    });
    assert_eq!(out, Ok(EmitOutcome::Disabled));
    assert_eq!(built.get(), 0);
    let out = bus.emit_with(SignalKind::Paused, |s| {
        built.set(built.get() + 1);
        event(s, paused())
    });
    assert_eq!(out, Ok(EmitOutcome::Queued));
    assert_eq!(built.get(), 1);
    assert_eq!(drain_all(&mut drain).events.len(), 1);
}

#[test]
fn counters_respect_the_toggles_too() {
    let (bus, mut drain, _) = setup(|_| {});
    bus.settings().set_category(SignalCategory::Browsing, false);
    for _ in 0..1_000 {
        let _ = bus.counters().impression(CounterEntity(1));
    }
    assert!(drain_all(&mut drain).rows.is_empty());
}

fn volume_tick(from: u8, to: u8) -> GestureTick<VolumePayload> {
    GestureTick {
        context: EventContext {
            session: SessionId::from_parts(at(), [2; 10]).unwrap(),
            recording: Some(RecordingId::from_parts(at(), [3; 10]).unwrap()),
            source: None,
            pos_ds: None,
            ctx: None,
        },
        payload: VolumePayload {
            from: Volume::from_raw(from),
            to: Volume::from_raw(to),
            gesture: GestureTime::from_ms(0),
            input: InputSource::Mouse,
            section: Section::UNKNOWN,
            since_track_start: ElapsedDs::from_ds(0),
            since_section_start: ElapsedDs::from_ds(0),
        },
    }
}

#[test]
fn forty_slider_ticks_in_500_ms_make_one_event_first_from_last_to() {
    let (bus, mut drain, clock) = setup(|_| {});
    let mut co = GestureCoalescer::new(bus.clone());
    for i in 0..40_u8 {
        assert_eq!(co.volume(|| volume_tick(i, i + 1)), TickOutcome::Buffered);
        clock.advance(12); // 40 ticks over 480 ms
    }
    clock.advance(600);
    assert_eq!(co.flush_due(), 1);
    let sink = drain_all(&mut drain);
    assert_eq!(sink.events.len(), 1);
    let SignalPayload::VolumeChanged(p) = sink.events[0].payload else {
        panic!("expected VolumeChanged");
    };
    assert_eq!((p.from, p.to), (Volume::from_raw(0), Volume::from_raw(40)));
}

#[test]
fn one_thousand_impressions_on_one_entity_are_one_counter_row_with_count_1000() {
    let (bus, mut drain, _) = setup(|_| {});
    for _ in 0..1_000 {
        let _ = bus.counters().impression(CounterEntity(77));
    }
    let sink = drain_all(&mut drain);
    assert_eq!(sink.rows.len(), 1);
    let row = sink.rows[0];
    assert_eq!(
        (row.kind, row.entity, row.count, row.day.yyyymmdd()),
        (SignalKind::Impression, CounterEntity(77), 1_000, 20_261_102)
    );
    assert!(sink.events.is_empty(), "impressions are never raw events");
}

#[test]
fn counter_kinds_are_refused_as_events() {
    let (bus, _drain, _) = setup(|_| {});
    let out = bus.emit_with(SignalKind::Impression, |_| unreachable!());
    assert_eq!(out, Err(SignalsError::NotAnEvent(SignalKind::Impression)));
}

proptest! {
    /// For any mix of reserved and low emissions and any small capacities, nothing is
    /// invented, duplicated or silently lost: reserved events all arrive (within capacity)
    /// and delivered + dropped accounts for every low event.
    #[test]
    fn every_emitted_event_is_delivered_or_counted_as_dropped(
        low_capacity in 1_usize..32,
        script in proptest::collection::vec(any::<bool>(), 0..400),
    ) {
        let (bus, mut drain, _) = setup(|c| {
            c.low_capacity = low_capacity;
            c.reserved_capacity = 512;
        });
        let (mut reserved, mut low) = (0_u64, 0_u64);
        for is_reserved in script {
            if is_reserved {
                reserved += 1;
                let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
            } else {
                low += 1;
                let _ = bus.emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()));
            }
        }
        let sink = drain_all(&mut drain);
        let got = |k| sink.events.iter().filter(|e| e.kind() == k).count() as u64;
        prop_assert_eq!(got(SignalKind::Paused), reserved);
        prop_assert_eq!(bus.dropped(SignalKind::Paused), 0);
        prop_assert_eq!(got(SignalKind::ViewOpened) + bus.dropped(SignalKind::ViewOpened), low);
        prop_assert!(got(SignalKind::ViewOpened) <= low_capacity as u64);
        let seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        prop_assert!(seqs.windows(2).all(|w| w[0] < w[1]), "ascending, unique");
    }
}
