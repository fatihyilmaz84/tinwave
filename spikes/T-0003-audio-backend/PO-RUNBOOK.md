# PO runbook - things only a human can verify (T-0003)

All commands run from `spikes/T-0003-audio-backend/` in PowerShell. Everything plays at -20 dBFS or quieter
(test files) or at the volume you pass on the command line. Nothing here changes system settings.

One-time setup:

```powershell
cd D:\dev\Tinwave-worktrees\T-0003-spike-audio-backend\spikes\T-0003-audio-backend
cargo build --release
.\target\release\t0003-audio-spike.exe gen        # writes ./out/*.wav|flac|mp3|m4a (about 30 MB, git-ignored)
.\target\release\t0003-audio-spike.exe devices    # lists render endpoints
```

Note from the agent run: on the dev machine the default endpoint was **muted at 6 % master volume**
(found while testing the system-volume hook). All agent measurements are digital (callback capture and WASAPI
loopback), so they do not depend on that. For the listening tests below, unmute or raise the volume first.

## 1. Device unplug while playing (USB DAC / headphones / HDMI monitor)

Goal: confirm the engine notices the loss, rebuilds on the new default device and resumes at the same position.

1. In Windows Settings > System > Sound, make the removable device the default output.
2. Start the follower with each backend, one after the other (60 s window):
   ```powershell
   .\target\release\t0003-audio-spike.exe follow wasapi 48000 60
   .\target\release\t0003-audio-spike.exe follow cpal   48000 60
   ```
3. While it plays the test sweep (a 10 s two-file loop, 40 repetitions), **unplug the device**.
4. Look for, in the console:
   - an `[notify] default render device ...` or `[notify] device ... state ...` line (WASAPI notification),
   - `event at ...s (backend lost=...)`, then `recovered in NN ms on [...]` and `resumed at track N MMMM ms`.
5. Expected: playback continues on the new default (or the console reports `recovery FAILED` if no endpoint is left,
   which is the correct "nothing to play to" behaviour). Report: recovery time, whether a click/half-second of
   silence was audible, whether the resumed position was within ~100 ms of where it stopped, and any exception text.
6. Repeat with the device **re-plugged** (should produce another notification; playback stays on the other device,
   which is expected - switching back is a product decision, not an engine one).

## 2. Bluetooth connect / disconnect

Same as section 1 but with Bluetooth headphones: start `follow wasapi 48000 90` on the speakers, then power on the
headphones. If Windows moves the default endpoint to them, expect `[notify] default render device` followed by
`recovered ...`. If it does not move the default, nothing should happen and playback must stay on the speakers.
Note the BT mix rate: `devices` prints it (BT A2DP endpoints are often 44.1 kHz or 48 kHz; the engine opens the
stream at the file rate and lets the OS convert in shared mode).

## 3. Change the default device by hand

Run `follow wasapi 48000 60`, then in Settings change the default output to another device and back.
Expected: two `recovered in ... ms` lines, position continuous, no repeat or skip of audio.

## 4. WASAPI exclusive mode with another app holding the device

a. With no other app playing: `.\target\release\t0003-audio-spike.exe excl 48000` - expected `exclusive OK ... 32b/24vb period 4000 us`
   (this already passed on the dev machine; it briefly takes the device for about 1.5 s, other apps go silent).
b. With a browser/music app playing in shared mode: same command - expected still OK (exclusive pre-empts shared).
c. With another app that itself holds the device in exclusive mode (e.g. a DAW, foobar2000 in WASAPI exclusive):
   same command - expected `exclusive stream FAILED: ... in use` (AUDCLNT_E_DEVICE_IN_USE). Report the text.

## 5. Listen to gapless on real albums (the real-world format test)

Pick a gapless album you know (live album, DJ mix, classical movements) and pass its files in order. All files
of one run must share a sample rate (the spike has no resampler between tracks).

```powershell
.\target\release\t0003-audio-spike.exe play wasapi 0.2 "D:\Music\Album\01.flac" "D:\Music\Album\02.flac" "D:\Music\Album\03.flac"
.\target\release\t0003-audio-spike.exe play cpal   0.2 "D:\Music\Album\01.mp3"  "D:\Music\Album\02.mp3"
.\target\release\t0003-audio-spike.exe play wasapi 0.2 "D:\Music\Album\01.m4a"  "D:\Music\Album\02.m4a"
```

It prints each file's codec, delay/padding as read by Symphonia, and logs track changes with the underrun count.
What to listen for at track boundaries: a click, a hole (silence), or a doubled beat. Please try at least one
iTunes/Apple-encoded `.m4a` album: the agent could only generate AAC with the Windows encoder (no iTunes-style
`iTunSMPB`/edit-list priming), and we expect an audible ~20-50 ms hole there until the library layer parses
`iTunSMPB`/`elst` and passes trim values (proved to work in the offline test, `aac ... ext-trim`).
Also try a VBR MP3 (no CBR) and seek in it by hand if you extend the spike: VBR seeking was not verified.

## 6. Optional: system-volume hook with the real keyboard keys

```powershell
.	argetelease	0003-audio-spike.exe sysvol 15
```

It nudges the master volume by 1 % and restores it (the agent already ran this: notification after 1.6 ms), then
listens for 15 more seconds. Press the keyboard volume keys and mute in that window. Expected: one line per press
with the new master volume and mute state. This is the `SystemVolumeChanged` / `MuteToggled` source.

## 7. Exclusive reopen at the file rate on a real DAC (T-0066)

Goal: measure what the exclusive-mode reopen costs on your DAC when an album goes from 44.1 kHz to 96 kHz, and see
how your DAC answers when Tinwave asks for 96 kHz exclusively (accepts, or refuses and Tinwave falls back to its own
48 kHz and resamples). Before T-0066 this was only ever run against fakes.

**What the test plays: digital silence only.** Two 0.4 s WAV files of all-zero samples (44.1 kHz, then 96 kHz), so
nothing audible comes out whatever the volume. It never changes the Windows volume, mute, default device or any other
setting. It briefly (about 1 to 2 seconds in all) opens your default output **exclusively**, so other applications
lose the device while it runs and get it back afterwards. It does not run in a normal `cargo test` / `cargo nextest
run`: it is marked `#[ignore]` and only runs when you ask for it as below.

Before you start:

1. Make the DAC under test the Windows **default output** (Settings > System > Sound). The test uses the default
   device; it does not pick one.
2. Close anything that may hold the DAC exclusively (foobar2000 in WASAPI exclusive, a DAW, ...), and pause other
   players.
3. Turn the amplifier or headphone level on the **device itself** down. DACs and amps can click when the sample rate
   switches even on a silent stream, and that click is not scaled by anything in software.

Run (PowerShell, from the repository root, not from `spikes\T-0003-audio-backend`; no Qt environment is needed):

```powershell
cargo test -p tinwave-playback --test device -- --ignored --nocapture --test-threads=1 exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported
```

The first run compiles for a minute or two; the test itself takes under 2 seconds. It prints a block like this (the
`<...>` are numbers from your machine):

```
wasapi exclusive reopen on "<your DAC's endpoint name>" (engine rate 48000 Hz, files 44100 Hz then 96000 Hz)
  open #0: asked 48000 Hz took <ms> ms -> Exclusive I24In32 | period <fr> fr | buffer <fr> fr | fallback None
  open #1: asked 44100 Hz (exclusive only) took <ms> ms -> Exclusive I24In32 | period ... | buffer ... | fallback None
  open #2: asked 96000 Hz (exclusive only) took <ms> ms -> Exclusive I24In32 | period ... | buffer ... | fallback None
  load -> first reopen announced: <ms> | load -> 44.1 kHz clip heard: <ms>
  boundary: reopen requested -> new stream announced: <ms> | last frame of the first clip -> first frame of the second: <ms>
  underrun events: <n>
test result: ok. 1 passed
```

How to read it:

- `open #0` is the start-up stream at Tinwave's own rate (48 kHz). `open #1` and `open #2` are the reopens for the
  44.1 kHz file (when it is loaded) and the 96 kHz file (at the boundary between the two clips). `took` is the whole
  `open` call: thread start, format probe, exclusive initialise, priming and starting the stream.
- `(exclusive only)` means Tinwave asked the output to fail instead of falling back to shared mode, so a refusal
  costs one failed open and no wasted shared stream.
- **The number that matters for the gap** is the last one on the `boundary:` line, "last frame of the first clip ->
  first frame of the second". It is measured by a loop that polls every ~1 ms, so it is good to a few ms. The app
  itself polls every 10 ms while a reopen is pending, which can add up to 10 ms on top. The spike measured about
  42 ms for the device buffer plus the exclusive open; compare.
- If the DAC does not accept 96 kHz exclusively, `open #2` prints `REFUSED, no stream opened: ... exclusive mode
  refused: ...` and `open #3` is `asked 48000 Hz` (not exclusive only): the 96 kHz file then plays resampled to 48 kHz.
  That is a valid outcome, not a test failure. Copy the refusal text.
- If the very first stream (`open #0`) is not exclusive, the test stops with `the endpoint ... does not open 48000 Hz
  exclusively, so there is no reopen to measure` and the reason. Report that text; it means exclusive mode is
  unavailable on that endpoint (disabled in the device's Advanced properties, or held by another application).

What to watch and listen for while it runs (the clips are silent, so you are checking the DAC, not the music):

- The DAC's sample-rate indicator (LED or display), if it has one: it should show 44.1 kHz and then 96 kHz (or stay at
  48 kHz after a refusal), and return to idle afterwards.
- A relay click, pop or thump from the amplifier or headphones at the switch from 44.1 to 96 kHz, and one when the
  test ends and the stream closes. Many DACs do this on every rate change; it is the DAC's own behaviour and tells you
  what a real album will sound like at the boundary.
- Windows should not switch the default device, and no other application should crash when it loses the device.

Please report: the whole printed block; the DAC model and connection (USB, optical, HDMI); whether the indicator
followed; whether you heard a click at the switch and at the end; and the Windows exclusive setting for the device if
you know it (Sound > the device > Properties > Advanced > "Allow applications to take exclusive control").

What this does **not** cover: hearing the gap between two real tracks. That needs the audible playback path of the app,
which does not exist yet, and the test is deliberately limited to silence. Until then the printed gap and the DAC's
own click are the evidence.
