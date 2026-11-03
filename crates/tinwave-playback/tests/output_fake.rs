//! The WASAPI callback logic (render, convert, write) driven by a fake device, through the real
//! engine: gapless and seek in every device format, exact resume after a device change, and
//! device-following decisions from fake notifications.
//!
//! No real audio device is opened by anything in this file.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::Arc;

use common::*;
use tinwave_core::{ManualClock, UtcMillis};
use tinwave_playback::{
    DeviceEvent, DeviceStateChange, Engine, EngineConfig, EventKind, FsOpener, QueueItemId,
    SampleFormat,
};

const BLOCK: usize = 480;
const FORMATS: [SampleFormat; 4] = [
    SampleFormat::F32,
    SampleFormat::I16,
    SampleFormat::I24In32,
    SampleFormat::I32,
];

struct Rig {
    engine: Engine,
    dev: FakeDeviceHandle,
    sink: Arc<CollectingSink>,
    format: SampleFormat,
}

fn rig(format: SampleFormat) -> Rig {
    let (output, dev) = FakeDeviceOutput::new(format, BLOCK);
    let sink = Arc::new(CollectingSink::default());
    let engine = Engine::start(
        EngineConfig {
            sample_rate: RATE,
            device: None,
            opener: Arc::new(FsOpener),
            sink: sink.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(1_000_000))),
        },
        Box::new(output),
    )
    .expect("engine starts");
    Rig {
        engine,
        dev,
        sink,
        format,
    }
}

impl Rig {
    /// Let the decoder carry out the last command. The acknowledgement is an empty render, so no
    /// audio of the new stream is consumed here.
    fn settle(&mut self) {
        for _ in 0..30_000 {
            if !self.engine.command_pending() {
                return;
            }
            self.dev.service_flush();
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder never carried out the command");
    }

    /// Everything the device receives from now on, `frames` frames of it, as f32.
    fn play(&mut self, frames: usize) -> Vec<f32> {
        self.settle();
        let _ = self.dev.take_recorded();
        let mut got = 0;
        while got < frames {
            for _ in 0..30_000 {
                if self.dev.buffered_frames() >= BLOCK {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
            assert!(self.dev.buffered_frames() >= BLOCK, "decoder starved");
            self.dev.step().expect("device open");
            self.engine.poll();
            got += BLOCK;
        }
        let mut out = decode_device_bytes(self.format, &self.dev.take_recorded());
        out.truncate(frames * 2);
        out
    }

    fn run_to_the_end(&mut self) {
        for _ in 0..30_000 {
            let _ = self.dev.step();
            self.engine.poll();
            if self.sink.kinds().contains(&EventKind::QueueEnded) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("QueueEnded never arrived");
    }
}

fn sources(
    tag: &str,
) -> Vec<(
    &'static str,
    std::path::PathBuf,
    std::path::PathBuf,
    TempDir,
)> {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    vec![
        ("wav16", a, b, dir),
        (
            "flac16",
            fixture("sweep_flac16_a.flac"),
            fixture("sweep_flac16_b.flac"),
            TempDir::new(&format!("{tag}-unused")),
        ),
        (
            "alac16",
            fixture("sweep_alac_a.m4a"),
            fixture("sweep_alac_b.m4a"),
            TempDir::new(&format!("{tag}-unused2")),
        ),
    ]
}

/// The T-0016 gapless test, through the output's callback logic in every device format: the
/// device receives the original sweep, bit for bit, across the A|B boundary.
#[test]
fn gapless_pair_reaches_the_device_bit_exactly_in_every_format() {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, 16);
    for (name, a, b, _dir) in sources("output-fake-gapless") {
        for format in FORMATS {
            let mut rig = rig(format);
            rig.engine
                .load(vec![track(1, &a), track(2, &b)], 0, 0)
                .expect("load");
            let rec = rig.play(TOTAL);
            assert_eq!(rec.len(), TOTAL * 2, "{name} {format:?}");
            assert_eq!(
                max_err(&rec, &ref_l, &ref_r, 0),
                0.0,
                "{name} {format:?} must be bit-exact"
            );
            let (rec_l, _) = deinterleave(&rec);
            assert_eq!(
                best_lag(&rec_l, &ref_l, SPLIT, 200, 4),
                0,
                "{name} {format:?}"
            );
            rig.run_to_the_end();
            assert_paired(&rig.sink.kinds(), false);
        }
    }
}

/// The T-0016 seek test, through the output's callback logic in every device format.
#[test]
fn seek_lands_on_the_requested_frame_at_the_device_in_every_format() {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, 16);
    for (name, a, b, _dir) in sources("output-fake-seek") {
        for format in FORMATS {
            let mut rig = rig(format);
            rig.engine
                .load(vec![track(1, &a), track(2, &b)], 0, 0)
                .expect("load");
            let _ = rig.play(7_200);
            for frame in [12_345u64, 3, 40_001, 47_000] {
                rig.engine.seek(frame).expect("seek");
                let rec = rig.play(1_440);
                assert_eq!(
                    max_err(&rec, &ref_l, &ref_r, frame as usize),
                    0.0,
                    "{name} {format:?} seek to {frame}"
                );
            }
            // Into B.
            rig.engine
                .load(vec![track(1, &a), track(2, &b)], 1, 5_001)
                .expect("load B");
            let rec = rig.play(1_440);
            assert_eq!(
                max_err(&rec, &ref_l, &ref_r, SPLIT + 5_001),
                0.0,
                "{name} {format:?} load into B"
            );
        }
    }
}

/// "Device re-open resumes at the same frame", from a fake notification: the default device
/// changes, the stream reports itself lost once, the host switches, and the music continues from
/// exactly the frame that was heard.
#[test]
fn a_default_device_change_reopens_and_resumes_at_the_same_frame() {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, 16);
    let (_name, a, b, _dir) = sources("output-fake-reopen").remove(0);
    let mut rig = rig(SampleFormat::I24In32);
    rig.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let before = rig.play(14_400);
    assert_eq!(max_err(&before, &ref_l, &ref_r, 0), 0.0);
    let heard = rig.engine.position().expect("position");

    // Noise that must not matter: a new device, another device's unplug, a capture default.
    for event in [
        DeviceEvent::Added { id: "usb".into() },
        DeviceEvent::StateChanged {
            id: "usb".into(),
            state: DeviceStateChange::Unplugged,
        },
        DeviceEvent::DefaultChanged {
            render: false,
            general_role: true,
            new_id: Some("mic".into()),
        },
    ] {
        rig.dev.notify(&event);
    }
    rig.engine.poll();
    assert!(!rig
        .sink
        .kinds()
        .iter()
        .any(|k| matches!(k, EventKind::OutputLost)));

    // The real thing: the default playback device moves to the headphones.
    rig.dev.notify(&DeviceEvent::DefaultChanged {
        render: true,
        general_role: true,
        new_id: Some("headphones".into()),
    });
    rig.engine.poll();
    rig.engine.poll();
    let lost = rig
        .sink
        .kinds()
        .iter()
        .filter(|k| matches!(k, EventKind::OutputLost))
        .count();
    assert_eq!(lost, 1, "reported once");

    // The host reacts: re-open on the (new) default.
    rig.engine.switch_device(None).expect("switch");
    assert_eq!(rig.dev.opens(), vec![None, None]);
    let after = rig.play(4_800);
    assert_eq!(
        max_err(&after, &ref_l, &ref_r, heard.frame as usize),
        0.0,
        "resumed at frame {}",
        heard.frame
    );
    assert_paired(&rig.sink.kinds(), true);
    assert!(!rig
        .sink
        .kinds()
        .iter()
        .any(|k| matches!(k, EventKind::TrackStarted { item: QueueItemId(1), cause, .. } if *cause != tinwave_playback::StartCause::Requested)));
}

/// A stream the user pinned to one device ignores default changes but follows its own device.
#[test]
fn a_pinned_device_follows_only_its_own_fate() {
    let (_name, a, _b, _dir) = sources("output-fake-pinned").remove(0);
    let (output, dev) = FakeDeviceOutput::new(SampleFormat::F32, BLOCK);
    let sink = Arc::new(CollectingSink::default());
    let mut engine = Engine::start(
        EngineConfig {
            sample_rate: RATE,
            device: Some("dac".into()),
            opener: Arc::new(FsOpener),
            sink: sink.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
        },
        Box::new(output),
    )
    .expect("engine starts");
    engine.load(vec![track(1, &a)], 0, 0).expect("load");
    dev.notify(&DeviceEvent::DefaultChanged {
        render: true,
        general_role: true,
        new_id: Some("headphones".into()),
    });
    engine.poll();
    assert!(!sink
        .kinds()
        .iter()
        .any(|k| matches!(k, EventKind::OutputLost)));
    dev.notify(&DeviceEvent::Removed { id: "dac".into() });
    engine.poll();
    assert!(sink
        .kinds()
        .iter()
        .any(|k| matches!(k, EventKind::OutputLost)));
}

/// The dither policy through the output's callback logic (T-0019): with a volume below 1 the
/// signal leaves the integer grid, and only the formats that quantise it audibly (I16, I24In32)
/// get dither. F32 and I32 carry the renderer's samples as they are.
#[test]
fn a_volume_below_one_is_dithered_only_on_the_integer_outputs_that_need_it() {
    let dir = TempDir::new("output-fake-dither");
    let (l, r) = sweep(RATE, TOTAL);
    let wav = dir.path("a.wav");
    write_wav(&wav, RATE, 16, &l[..SPLIT], &r[..SPLIT]);

    // Skip the first block: the volume ramps from 1.0 to 0.3 across it.
    let (skip, frames) = (BLOCK, 4 * BLOCK);
    let play = |format: SampleFormat| -> Vec<f32> {
        let mut rig = rig(format);
        rig.engine.set_volume(0.3).expect("volume");
        rig.engine.load(vec![track(1, &wav)], 0, 0).expect("load");
        let mut got = rig.play(frames);
        got.drain(..skip * 2);
        got
    };
    let float = play(SampleFormat::F32);
    assert!(
        float.iter().any(|&s| s != 0.0),
        "the test signal must not be silent"
    );

    // I32 is converted without dither: the f32 samples come back unchanged.
    let wide = play(SampleFormat::I32);
    assert!(float.iter().zip(&wide).all(|(a, b)| (a - b).abs() < 1e-7));

    for (format, scale) in [
        (SampleFormat::I16, 32_768.0_f32),
        (SampleFormat::I24In32, 8_388_608.0_f32),
    ] {
        let got = play(format);
        assert_eq!(got.len(), float.len());
        let (mut differs, mut worst) = (0usize, 0.0f32);
        for (g, f) in got.iter().zip(&float) {
            let plain = (f * scale).round();
            let step = (g * scale - plain).abs();
            differs += usize::from(step > 0.5);
            worst = worst.max(step);
        }
        assert!(
            differs > got.len() / 10,
            "{format:?}: dither must move many samples off plain rounding, moved {differs} of {}",
            got.len()
        );
        assert!(
            worst <= 1.01,
            "{format:?}: dither is at most one step, got {worst}"
        );
    }
}
