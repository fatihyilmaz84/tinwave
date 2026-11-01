SPIKE RESULT T-0003
answer: Use Symphonia 0.6 for decode plus our own backend-independent playback engine (decoder thread -> lock-free ring -> RT renderer), with direct WASAPI output on Windows (shared by default, exclusive for ExclusiveModeToggled) and cpal as the fallback and the macOS/Linux output; the gaps are M4A/AAC gapless (Symphonia ignores edit lists and iTunSMPB, so we must parse and apply it ourselves) and a resampler (rubato).
confidence: high for Windows (measured). Medium overall: iTunes-style M4A, VBR MP3, libFLAC files, HE-AAC and Opus were not testable here, and device unplug and Bluetooth need the PO.

evidence:
- Machine: Windows 11 Pro 10.0.26200, Ryzen 7 7800X3D, Rust 1.92 release build. Default endpoint "High Definition Audio Device": 48 kHz / 2 ch / f32, 10 ms period. 4 render endpoints were present at the start, 3 at the end (the monitor endpoint disappeared on its own).
- Test material: generated -20 dBFS log sweep (200 Hz to 12 kHz, 10 s), split into 2 files at frame 5*rate+777 (not codec-aligned). Formats: WAV 16/24/f32, FLAC 16/24, MP3 (LAME tag and no tag), ALAC, AAC-LC, at 48 and 44.1 kHz. ffmpeg is NOT on PATH, so AAC was made with the Windows Media Foundation encoder and ALAC with the alac-encoder crate plus my own small MP4 muxer.
- Gapless was measured three ways and they agree: offline decode, the engine's callback recording (24 runs), and WASAPI loopback capture of the OS mixer (cpal, direct WASAPI and rodio).
  - 0-frame boundary gap, bit-exact for lossless: WAV, FLAC, ALAC, and MP3 with a LAME tag.
  - MP3 without a tag: +1059 to +2295 frames (24 to 48 ms).
  - AAC from the Windows MF encoder: +887 frames (18.5 ms) at 48 kHz and +931 at 44.1 kHz. With an external trim passed to the reader it goes to 0.
- Symphonia 0.6.1 gapless support, checked in source and README: MP3 trims from the LAME tag. "ISO/MP4 gapless: No" and "AAC-LC gapless: No"; `edts/elst` is parsed but never used, and there is no iTunSMPB handling. Symphonia's AAC is LC only (no HE-AAC/SBR).
- Seek: after a seek the recorded output matches the reference sweep from exactly the requested frame. Max error is 1.5e-5 for lossless and about 1e-3 to 4e-3 for AAC (lossy codec noise). This held for 5 formats on both backends and for offline seeks at 5 positions per format. Seek to first audio in the callback is 0.5 to 1.3 ms, or about 11 ms when the seek lands just after a callback.
- Position at 10 Hz is monotonic in all runs. It is quantised to the device buffer: step-vs-wall deviation is 12-14 ms for cpal and 22-25 ms for direct WASAPI. Four runs near an untrimmed track boundary (MP3 without tag and AAC without trim, both backends) showed single deviations of 32-95 ms; I did not investigate the cause. Seek landing itself is sample exact.
- Volume: atomic target with a linear per-block ramp; the gain measured in the recording is 0.250 and 1.000 as set. 0 heap allocations or frees inside `Renderer::render` over all runs including seeks and volume changes (counting global allocator).
- Cold start (Engine::start to first real audio callback): direct WASAPI median 18 ms (17 to 24, no outliers). cpal median 28 ms (23 to 34) with two outliers at 105 and 304 ms. Output latency to the speaker is 22 ms for WASAPI and 32 ms for cpal (device-reported). Underruns: 0 in all runs.
- CPU is under 1% of one core on both backends. This is limited by the 15.6 ms resolution of `GetProcessTimes`, so I only claim the bound. Whole spike process working set is 11.5 to 14.1 MB (about 3 MB private over idle), with no Qt. Decode speed on one thread (40 s stereo): WAV 5200x realtime, FLAC 1700x, AAC 1600x, MP3 1200x, ALAC 640x.
- Device switch (stop, open, first audio, resume at the same frame): 14 to 25 ms on real or virtual endpoints, but 113 ms and 574 ms the first time a virtual endpoint was opened.
- Exclusive mode is feasible. The endpoint accepts i16, i24-in-32 and i32 at 44.1 to 192 kHz, but not f32 or packed 24-bit. Opened 24-in-32 at 48 kHz with a 4 ms period, 42 ms to first audio, position exact. cpal 0.18 has no exclusive support.
- Signal hooks confirmed. `IAudioEndpointVolume::RegisterControlChangeNotify` fires 1.6 ms after a change (a SystemVolumeChanged source). Registration of IMMNotificationClient default-device and state callbacks works, but I could not trigger them. The engine's rebuild-on-new-device path (`switch_device`) is verified.
- rubato Fft 44.1 to 48 kHz runs at about 2000x realtime. Per-track independent resampling keeps the exact length but introduces under 1 output frame of phase error at the boundary; use a continuous resampler across same-rate tracks.
- The default endpoint was muted at 6% master volume when I tested, so my tests were inaudible. All measurements are digital and unaffected.

options compared:
- Symphonia + own engine + direct WASAPI (wasapi 0.26) — recommended on Windows — same gapless/seek results as cpal, 10 ms lower latency, stable start, exclusive mode, real device and endpoint-volume notifications.
- Symphonia + own engine + cpal 0.18 — recommended for macOS/Linux and as a Windows fallback — passed every test; shared-mode only, higher and jumpier start, no volume or device notifications, no exclusive.
- rodio 0.23 (Symphonia + cpal) — rejected — gapless measured fine by loopback, but `Source::next()` (file I/O and decoding) runs inside the cpal data callback, which violates the no-blocking/no-allocation rule. ALAC is not in its default features, and it has no exclusive mode or device events.
- Not evaluated: miniaudio (C), FFmpeg/libmpv (LGPL/GPL, large), BASS (commercial), Media Foundation source reader (candidate AAC/HE-AAC decode fallback, OS-licensed).

licenses:
- symphonia 0.6.1 — MPL-2.0 — file-level copyleft, fine if unmodified; software AAC decoding has no patent license attached (Via LA pool is active).
- cpal 0.18.2 — Apache-2.0 — no concern.
- wasapi 0.26 — MIT — no concern.
- windows and windows-core 0.62, rtrb 0.4, rubato 5 (with audioadapter), rodio 0.23 — MIT OR Apache-2.0 — no concern.
- hound, flacenc, alac-encoder — Apache-2.0 / Apache-2.0 / Apache-2.0 OR MIT — test-file generation only.
- mp3lame-encoder 0.2.5 (LAME) — LGPL-3.0 — FLAG: test generation only; it must not enter the shipped dependency tree.
- Codec patents: MP3 patents are expired (US last core patent April 2017; Fraunhofer/Technicolor licensing ended 2017-04-23). FLAC and ALAC are unencumbered. For AAC, Via LA's pool is still active and charges for decoders sold in products; bitstreams are free. This needs a PO/legal decision, or use the Windows built-in AAC decoder.

risks:
- M4A/AAC gapless needs our own iTunSMPB/elst parsing in the scanner; I could not reproduce iTunes-style 1105/2112-frame head priming (no ffmpeg or iTunes files here), so the head-gap case is unmeasured.
- MP3 without a LAME tag always has a 24 to 48 ms gap. VBR MP3 seeking was not tested (my files are CBR).
- Symphonia refused flacenc files because of a non-standard STREAMINFO (`min_blocksize` set to the short last block). I patched this in the generator. libFLAC files are untested, so the scanner must tolerate odd encoders.
- HE-AAC is unsupported and Opus has no decoder; both are untested. ASIO, 5.1 output (the engine is stereo only), and 24-bit exclusive audible quality are untested.
- Exclusive mode needs a float-to-integer policy (dither); the spike truncates. A rate change between tracks needs the rubato path wired into the engine; the spike skips mismatched-rate tracks and relies on the OS converter in shared mode.
- Position is quantised to the buffer (10 to 20 ms). `IAudioClock` interpolation is possible on direct WASAPI but not built.
- Unplug, Bluetooth, user default-device change, and exclusive-vs-another-exclusive app are unverified — runbook provided. I deliberately did not change the system default device on the PO's machine.
- 3 of the 4 earliest commits lack the Co-Authored-By trailer (the rule reached me late); only the last carries it.

draft ADR: spikes/T-0003-audio-backend/ADR-draft.md (written as ADR 0004; it lists the spec and doc edits for you to apply on acceptance).
PO runbook: spikes/T-0003-audio-backend/PO-RUNBOOK.md
Raw results: spikes/T-0003-audio-backend/results/ (gapless, loopback, loopback_rodio, seekvol, probe, cpu, switch_*, exclusive, decspeed, resamp, sysvol)
Prototype: spikes/T-0003-audio-backend/ (standalone Cargo.toml with empty `[workspace]`). Commits on t/T-0003-spike-audio-backend (4 commits, nothing pushed).

follow-up tickets suggested:
- P0: Build `tinwave-playback` engine core from the spike design (reader, ring, renderer, seek handshake, position marks, device reconfigure) with the gapless sweep test and RT-allocation counter test as CI tests | this is the contract everything else depends on | size: M
- P0: Windows output module on direct WASAPI (shared + exclusive, MMCSS, IMMNotificationClient, IAudioEndpointVolume callback), with cpal as the fallback and generic backend | feeds the device and ExclusiveModeToggled/SystemVolumeChanged signals | size: M
- P1: Gapless metadata extraction in the scanner: M4A iTunSMPB and elst, MP3 LAME/Xing, stored per file and passed to the reader | closes the AAC head and tail gap | size: S
- P1: Resampler integration (rubato Fft in the decoder thread, one continuous resampler across same-rate tracks, flush on rate change) and the exclusive-mode float-to-int/dither policy | needed for 44.1 libraries on 48 kHz endpoints and for rate changes between tracks | size: M
- P1: Spike: real-world format and device corpus — iTunes and ffmpeg M4A, libFLAC, VBR MP3, HE-AAC, Opus/OGG/AIFF/WavPack, plus Media Foundation as an AAC/HE-AAC decode fallback | I had no ffmpeg or real encoders, and this also settles the AAC patent question | size: M
- P2: Sub-callback position interpolation through IAudioClock::GetPosition, if signals need better than 10 to 20 ms | optional | size: S

questions for lead / PO:
- AAC decoding: accept the Via LA exposure of a pure-Rust software AAC decoder, or route AAC on Windows through the OS Media Foundation decoder?
- Is a 24 to 48 ms gap acceptable for MP3 files without a LAME/Xing tag, or do we want a heuristic (assume the 1105-frame LAME delay)?
- Please run `PO-RUNBOOK.md` sections 1, 2 and 4c (unplug, Bluetooth, exclusive vs another exclusive app) and section 5 with one iTunes-style `.m4a` album.
- The default Windows output on the dev machine was muted at 6% master volume; the PO should unmute it before the listening tests.

---
Lead review (2026-11-02):
- Branch touches only spikes/T-0003-audio-backend/ (31 files, no audio binaries); merged as a prototype.
- Draft ADR stays Proposed (to become ADR 0005, not 0004 — 0004 is taken) until the PO answers D-0003 (MPL-2.0 for
  Symphonia + spec §2/§7 change) and D-0004 (AAC decode route).
- Lead decision: no heuristic for untagged MP3 — the 24–48 ms gap stays (a guessed trim could cut real audio).
- Lead decision: playback output keeps its own IMMNotificationClient for stream re-open only; all signals stay in
  tinwave-context (ADR 0004 / T-0014), so a user disabling T1 device context never breaks device following.
- Follow-ups: T-0016 engine core, T-0017 Windows output, T-0018 gapless metadata in scanner, T-0019 resampler + dither,
  T-0020 real-world corpus spike; IAudioClock interpolation parked (P-0027); T-0009 unblocked.
