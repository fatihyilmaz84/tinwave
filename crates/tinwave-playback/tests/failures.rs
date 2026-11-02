//! Errors are reported, not swallowed: corrupt packets, tracks that fail, impossible seeks,
//! unsupported layouts, and the one-`TrackLeft`-per-`TrackStarted` pairing the signal layer
//! relies on.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use common::*;
use symphonia::core::io::MediaSource;
use tinwave_playback::{
    EventKind, GaplessInfo, LeaveCause, PlaybackError, QueueItemId, SourceOpener, StartCause, Track,
};

/// A 3 s tag-less MP3 made of the fixture's audio frames three times over, with the side info of
/// three frames near the start damaged. Long enough that the decoder is still reading when the ring
/// (1 s) is full. CBR 192 kbps at 48 kHz: 576 bytes per frame, the first (Xing/LAME) frame is
/// dropped. Damage: the first granule's `big_values` field (side-info bits 32 to 40, after the
/// 4-byte header) is set to 511; the decoder rejects values above 288 with a decode error, which
/// Symphonia reports per packet.
fn corrupt_mp3(dir: &TempDir) -> std::path::PathBuf {
    let original = std::fs::read(fixture("sweep_mp3lame_a.mp3")).expect("read fixture");
    let audio = &original[576..];
    let mut bytes = [audio, audio, audio].concat();
    for frame in 9..12 {
        let side_info = frame * 576 + 4;
        bytes[side_info + 4] = 0xFF;
        bytes[side_info + 5] |= 0x80;
    }
    let path = dir.path("corrupt.mp3");
    std::fs::write(&path, bytes).expect("write");
    path
}

fn corrupt_events(h: &Harness) -> Vec<(QueueItemId, u64)> {
    h.sink
        .kinds()
        .into_iter()
        .filter_map(|k| match k {
            EventKind::CorruptPackets { item, packets } => Some((item, packets)),
            _ => None,
        })
        .collect()
}

/// Loads `[corrupt A, B]` and waits until the decoder is blocked on a full ring somewhere in A,
/// past the damage but before the track's end, so the reader is still alive when the next command comes.
fn start_with_damage(h: &mut Harness, dir: &TempDir) -> Vec<Track> {
    let queue = vec![
        track(1, corrupt_mp3(dir)),
        track(2, fixture("sweep_mp3lame_b.mp3")),
    ];
    h.engine.load(queue.clone(), 0, 0).expect("load");
    h.settle();
    h.wait_buffered(44_000);
    // Give the decoder time to hit the full ring (it reads ahead of what we waited for).
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(corrupt_events(h).is_empty(), "reader must still be open");
    queue
}

#[test]
fn corrupt_packets_are_reported_at_the_natural_end_of_the_track() {
    let dir = TempDir::new("corrupt-end");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, corrupt_mp3(&dir))], 0, 0)
        .expect("load");
    h.settle();
    let mut ended = false;
    for _ in 0..30_000 {
        let _ = h.render_block(480);
        if h.sink.kinds().contains(&EventKind::QueueEnded) {
            ended = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    assert!(ended, "queue never ended");
    let found = corrupt_events(&h);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, QueueItemId(1));
    assert!(found[0].1 > 0);
    // Skipped packets are a glitch, not a failure: the track still counts as finished.
    assert!(h.sink.kinds().iter().any(|k| matches!(
        k,
        EventKind::TrackLeft {
            item: QueueItemId(1),
            cause: LeaveCause::Finished,
            ..
        }
    )));
}

#[test]
fn corrupt_packets_are_reported_when_the_track_is_skipped() {
    let dir = TempDir::new("corrupt-skip");
    let mut h = Harness::new();
    let queue = start_with_damage(&mut h, &dir);
    h.engine.load(queue, 1, 0).expect("skip");
    h.settle();
    let found = corrupt_events(&h);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, QueueItemId(1));
    assert!(found[0].1 > 0);
}

#[test]
fn corrupt_packets_are_reported_when_seeking_inside_the_track() {
    let dir = TempDir::new("corrupt-seek");
    let mut h = Harness::new();
    let _queue = start_with_damage(&mut h, &dir);
    h.engine.seek(3_000).expect("seek");
    h.settle();
    let found = corrupt_events(&h);
    assert_eq!(
        found.len(),
        1,
        "the count read so far survives the seek: {found:?}"
    );
    assert_eq!(found[0].0, QueueItemId(1));
}

#[test]
fn corrupt_packets_are_reported_when_the_device_is_switched() {
    let dir = TempDir::new("corrupt-switch");
    let mut h = Harness::new();
    let _queue = start_with_damage(&mut h, &dir);
    h.engine
        .switch_device(Some("other".into()))
        .expect("switch");
    h.settle();
    let found = corrupt_events(&h);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, QueueItemId(1));
}

/// A WAV that reads fine up to `fail_after` bytes and then returns an IO error.
struct BrokenSource {
    inner: Cursor<Vec<u8>>,
    delivered: usize,
    fail_after: usize,
}

impl Read for BrokenSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.delivered >= self.fail_after {
            return Err(std::io::Error::other("disk went away"));
        }
        let n = self.inner.read(buf)?;
        self.delivered += n;
        Ok(n)
    }
}

impl Seek for BrokenSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for BrokenSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.inner.get_ref().len() as u64)
    }
}

/// Opens the named file normally, except `broken` which fails mid-stream.
#[derive(Debug)]
struct BreakingOpener {
    broken: std::path::PathBuf,
    fail_after: usize,
}

impl SourceOpener for BreakingOpener {
    fn open(&self, track: &Track) -> std::io::Result<Box<dyn MediaSource>> {
        let bytes = std::fs::read(&track.path)?;
        let fail_after = if track.path == self.broken {
            self.fail_after
        } else {
            usize::MAX
        };
        Ok(Box::new(BrokenSource {
            inner: Cursor::new(bytes),
            delivered: 0,
            fail_after,
        }))
    }
}

fn wav_files(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, TempDir) {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    (a, b, dir)
}

#[test]
fn a_track_failing_mid_decode_is_left_as_failed_not_finished() {
    let (a, b, _dir) = wav_files("fail-mid");
    let opener = Arc::new(BreakingOpener {
        broken: a.clone(),
        fail_after: 60_000,
    });
    let mut h = Harness::with_opener(opener);
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    // Whatever A delivered before breaking, then all of B.
    h.settle();
    h.wait_buffered(10_000);
    let mut heard = 0usize;
    while !h.sink.kinds().iter().any(|k| {
        matches!(
            k,
            EventKind::TrackLeft {
                item: QueueItemId(2),
                ..
            }
        )
    }) {
        h.wait_buffered(480.min(h.out.buffered_frames().unwrap_or(0).max(1)));
        let (_, real) = h.render_block(480);
        heard += real;
        if heard > 3 * TOTAL {
            panic!("track B never ended");
        }
        if real == 0 {
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
    }
    let kinds = h.sink.kinds();
    assert!(
        kinds.iter().any(|k| matches!(
            k,
            EventKind::TrackFailed {
                item: QueueItemId(1),
                error: PlaybackError::Decode(_)
            }
        )),
        "{kinds:?}"
    );
    let left_a: Vec<&EventKind> = kinds
        .iter()
        .filter(|k| {
            matches!(
                k,
                EventKind::TrackLeft {
                    item: QueueItemId(1),
                    ..
                }
            )
        })
        .collect();
    assert_eq!(left_a.len(), 1, "{kinds:?}");
    assert!(
        matches!(left_a[0], EventKind::TrackLeft { cause: LeaveCause::Failed, frame, .. } if *frame > 0 && *frame < SPLIT as u64),
        "{left_a:?}"
    );
    // B then finishes normally.
    assert!(kinds.contains(&EventKind::TrackLeft {
        item: QueueItemId(2),
        frame: (TOTAL - SPLIT) as u64,
        cause: LeaveCause::Finished
    }));
    assert_paired(&kinds, false);
}

#[test]
fn the_last_track_failing_mid_decode_ends_the_queue_as_failed() {
    let (a, _b, _dir) = wav_files("fail-last");
    let opener = Arc::new(BreakingOpener {
        broken: a.clone(),
        fail_after: 60_000,
    });
    let mut h = Harness::with_opener(opener);
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    h.settle();
    h.wait_buffered(10_000);
    for _ in 0..30_000 {
        let (_, _) = h.render_block(480);
        if h.sink.kinds().contains(&EventKind::QueueEnded) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    let kinds = h.sink.kinds();
    assert!(kinds.contains(&EventKind::QueueEnded), "{kinds:?}");
    assert!(kinds.iter().any(|k| matches!(
        k,
        EventKind::TrackLeft {
            item: QueueItemId(1),
            cause: LeaveCause::Failed,
            ..
        }
    )));
    assert!(!kinds.iter().any(|k| matches!(
        k,
        EventKind::TrackLeft {
            cause: LeaveCause::Finished,
            ..
        }
    )));
    assert_paired(&kinds, false);
}

#[test]
fn a_track_that_fails_to_open_after_load_still_gets_a_track_left() {
    let (_a, b, dir) = wav_files("fail-open");
    let garbage = dir.path("garbage.wav");
    std::fs::write(&garbage, vec![0x55u8; 4_096]).expect("write");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &garbage), track(2, &b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(TOTAL - SPLIT, 480);
    h.finish();
    let kinds: Vec<EventKind> = h
        .sink
        .kinds()
        .into_iter()
        .filter(|k| {
            !matches!(
                k,
                EventKind::PositionMark { .. } | EventKind::Underrun { .. }
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::TrackStarted {
                item: QueueItemId(1),
                frame: 0,
                cause: StartCause::Requested
            },
            EventKind::TrackFailed {
                item: QueueItemId(1),
                error: PlaybackError::Probe(match &kinds[1] {
                    EventKind::TrackFailed {
                        error: PlaybackError::Probe(m),
                        ..
                    } => m.clone(),
                    other => panic!("unexpected {other:?}"),
                })
            },
            EventKind::TrackLeft {
                item: QueueItemId(1),
                frame: 0,
                cause: LeaveCause::Failed
            },
            EventKind::TrackStarted {
                item: QueueItemId(2),
                frame: 0,
                cause: StartCause::Advance
            },
            EventKind::TrackLeft {
                item: QueueItemId(2),
                frame: (TOTAL - SPLIT) as u64,
                cause: LeaveCause::Finished
            },
            EventKind::QueueEnded,
        ]
    );
    assert_paired(&h.sink.kinds(), false);
}

#[test]
fn a_queue_of_only_unopenable_tracks_closes_the_announced_track_as_failed() {
    let mut h = Harness::new();
    let missing = std::env::temp_dir().join("tinwave-playback-does-not-exist/x.wav");
    h.engine.load(vec![track(1, missing)], 0, 0).expect("load");
    h.settle();
    h.engine.poll();
    let kinds = h.sink.kinds();
    assert!(kinds.contains(&EventKind::TrackLeft {
        item: QueueItemId(1),
        frame: 0,
        cause: LeaveCause::Failed
    }));
    assert!(kinds.contains(&EventKind::QueueEnded));
    assert_paired(&kinds, false);
}

/// Seeking at or past the end of the track is reported as `SeekFailed(SeekOutOfRange)` for every
/// container, whether or not the demuxer notices by itself, and the queue moves on.
#[test]
fn seek_past_the_end_is_reported_for_wav_flac_alac_and_mp3() {
    let dir = TempDir::new("seek-end");
    let (l, r) = sweep(RATE, TOTAL);
    let wav_a = dir.path("a.wav");
    write_wav(&wav_a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    let cases: Vec<(&str, std::path::PathBuf)> = vec![
        ("wav", wav_a),
        ("flac", fixture("sweep_flac16_a.flac")),
        ("alac", fixture("sweep_alac_a.m4a")),
        ("mp3", fixture("sweep_mp3lame_a.mp3")),
    ];
    for (name, a) in cases {
        for target in [SPLIT as u64, SPLIT as u64 + 5_000, 10_000_000] {
            let mut h = Harness::new();
            h.engine
                .load(
                    vec![track(1, &a), track(2, fixture("sweep_flac16_b.flac"))],
                    0,
                    0,
                )
                .expect("load");
            let _ = h.render_exact(480, 480);
            h.engine.seek(target).expect("seek is accepted");
            h.settle();
            let kinds = h.sink.kinds();
            assert!(
                kinds.iter().any(|k| matches!(
                    k,
                    EventKind::SeekFailed {
                        item: QueueItemId(1),
                        error: PlaybackError::SeekOutOfRange { .. },
                        ..
                    }
                )),
                "{name} seek to {target}: {kinds:?}"
            );
            // The queue moves on to B, and the seeked track is closed as failed, not finished.
            let _ = h.render_exact(480, 480);
            let events = h.sink.kinds();
            assert!(
                events.iter().any(|k| matches!(
                    k,
                    EventKind::TrackStarted {
                        item: QueueItemId(2),
                        cause: StartCause::Advance,
                        ..
                    }
                )),
                "{name} seek to {target}: {events:?}"
            );
            assert!(
                events.iter().any(|k| matches!(
                    k,
                    EventKind::TrackLeft {
                        item: QueueItemId(1),
                        cause: LeaveCause::Failed,
                        ..
                    }
                )),
                "{name} seek to {target}: {events:?}"
            );
            assert_paired(&events, true);
        }
    }
}

/// The last valid frame is still seekable (the bound is exact, not conservative).
#[test]
fn seek_to_the_last_frame_works_in_every_container() {
    let dir = TempDir::new("seek-last");
    let (l, r) = sweep(RATE, TOTAL);
    let wav_a = dir.path("a.wav");
    write_wav(&wav_a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    let cases: Vec<(&str, std::path::PathBuf)> = vec![
        ("wav", wav_a),
        ("flac", fixture("sweep_flac16_a.flac")),
        ("alac", fixture("sweep_alac_a.m4a")),
        ("mp3", fixture("sweep_mp3lame_a.mp3")),
    ];
    for (name, a) in cases {
        let mut h = Harness::new();
        h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
        let _ = h.render_exact(480, 480);
        h.sink.take();
        h.engine.seek(SPLIT as u64 - 1).expect("seek");
        h.settle();
        let kinds = h.sink.kinds();
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, EventKind::SeekFailed { .. })),
            "{name}: {kinds:?}"
        );
        h.wait_buffered(1);
    }
}

#[test]
fn more_than_two_channels_is_a_reported_failure() {
    let dir = TempDir::new("three-channels");
    let (l, r) = sweep(RATE, TOTAL);
    let surround = dir.path("three.wav");
    write_wav_channels(&surround, RATE, 16, 3, &l[..4_800], &r[..4_800]);
    let good = dir.path("good.wav");
    write_wav(&good, RATE, 16, &l[..4_800], &r[..4_800]);
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &surround), track(2, &good)], 0, 0)
        .expect("load");
    let _ = h.render_exact(4_800, 480);
    h.finish();
    let kinds = h.sink.kinds();
    assert!(
        kinds.iter().any(|k| matches!(
            k,
            EventKind::TrackFailed {
                item: QueueItemId(1),
                error: PlaybackError::UnsupportedChannels { channels: 3 }
            }
        )),
        "{kinds:?}"
    );
    assert_paired(&kinds, false);
}

/// External gapless info (what T-0018 will extract from `iTunSMPB` / edit lists) trims the head
/// and the tail, and seeks, positions and lengths all live on the trimmed timeline.
#[test]
fn external_gapless_info_trims_head_and_tail_and_seeks_inside_the_trimmed_timeline() {
    let (l64, r64) = sweep(RATE, TOTAL);
    let refs = reference(&l64, &r64, 16);
    for (name, path) in [
        ("flac", fixture("sweep_flac16_a.flac")),
        ("alac", fixture("sweep_alac_a.m4a")),
    ] {
        let trimmed = Track {
            id: QueueItemId(1),
            path,
            gapless: Some(GaplessInfo {
                delay_frames: 1_000,
                valid_frames: 5_000,
            }),
        };
        let mut h = Harness::new();
        h.engine.load(vec![trimmed.clone()], 0, 0).expect("load");
        // Exactly the valid length, starting `delay` frames into the file.
        let rec = h.render_exact(5_000, 480);
        h.finish();
        assert_eq!(max_err(&rec, &refs.0, &refs.1, 1_000), 0.0, "{name}");
        assert!(
            h.sink.kinds().contains(&EventKind::TrackLeft {
                item: QueueItemId(1),
                frame: 5_000,
                cause: LeaveCause::Finished
            }),
            "{name}: {:?}",
            h.sink.kinds()
        );

        // Seek inside the trimmed timeline: frame 2_000 is file frame 3_000. A seek to the end of
        // the valid range is out of range even though the file continues.
        h.engine.load(vec![trimmed.clone()], 0, 0).expect("reload");
        h.settle();
        let _ = h.render_exact(480, 480);
        h.engine.seek(2_000).expect("seek");
        h.settle();
        let rec = h.render_exact(1_500, 500);
        assert_eq!(max_err(&rec, &refs.0, &refs.1, 3_000), 0.0, "{name} seek");
        h.sink.take();
        h.engine.seek(5_000).expect("seek accepted");
        h.settle();
        assert!(
            h.sink.kinds().iter().any(|k| matches!(
                k,
                EventKind::SeekFailed {
                    error: PlaybackError::SeekOutOfRange { .. },
                    ..
                }
            )),
            "{name}"
        );
    }
}
