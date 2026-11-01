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
