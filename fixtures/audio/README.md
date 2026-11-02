# Audio fixtures

Self-made test audio for `tinwave-playback` (gapless and seek tests, ADR 0005). It is a computer-generated
sine sweep: no recording, no copyrighted material. **CC0 / public domain**; use it for anything.

## Content

One 2.000 s stereo sweep at 48 kHz, split into two files **A | B** at frame `48000 + 777 = 48777`. The split is
deliberately not aligned to any codec frame (MP3 1152, ALAC 4096, FLAC block 4096), so only a correct
gapless hand-off plays the pair back as the original 96 000 frames.

- Left channel: exponential sine sweep 200 Hz to 12 kHz over the full 2 s, amplitude 0.1 (-20 dBFS).
- Right channel: the same sweep at half amplitude.
- The sweep is a pure function of the parameters, so the tests regenerate the reference in code
  (`crates/tinwave-playback/tests/common/mod.rs`, `sweep`) instead of storing it.

| File | Format | Encoder |
|---|---|---|
| `sweep_flac16_a.flac`, `sweep_flac16_b.flac` | FLAC, 16 bit | `flacenc` 0.5 (Apache-2.0), STREAMINFO `min_blocksize` patched like libFLAC writes it |
| `sweep_alac_a.m4a`, `sweep_alac_b.m4a` | ALAC in MP4, 16 bit | `alac-encoder` 0.3 plus the generator's own minimal MP4 muxer |
| `sweep_mp3lame_a.mp3`, `sweep_mp3lame_b.mp3` | MP3 CBR 192 kbps, **with LAME/Xing tag** (gapless header) | LAME via `mp3lame-encoder` 0.2.5 (LGPL-3.0, **generator only; never a Tinwave dependency**) |

Total size: about 200 KB. WAV pairs (16 bit, 24 bit, float32) are **not** stored: the tests write them at run
time with a 30-line WAV writer in `tests/common/mod.rs` (no `hound` dev-dependency).

## How they were made

With the generator from spike T-0003, `spikes/T-0003-audio-backend/src/gen.rs` (commit `80e3fcb`), copied into a
throwaway crate outside this workspace with dependencies `hound 3.5`, `flacenc 0.5`, `mp3lame-encoder 0.2`,
`alac-encoder 0.3`, `anyhow 1`, with the one line that calls the Windows Media Foundation AAC writer deleted.
The driver:

```rust
mod gen; // gen.rs from the spike
fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let rate = 48000u32;
    let total = rate as usize * 2;      // 2.000 s
    let split = rate as usize + 777;    // 48777, not codec aligned
    let (l, r) = gen::sweep(rate, total, 200.0, 12000.0);
    let (la, lb) = l.split_at(split);
    let (ra, rb) = r.split_at(split);
    for (part, ll, rr) in [("a", la, ra), ("b", lb, rb)] {
        gen::write_mp3(&out.join(format!("sweep_mp3lame_{part}.mp3")), rate, ll, rr, true)?;
        gen::write_flac(&out.join(format!("sweep_flac16_{part}.flac")), rate, 16, ll, rr)?;
        gen::write_alac_m4a(&out.join(format!("sweep_alac_{part}.m4a")), rate, ll, rr)?;
    }
    Ok(())
}
```

run as `cargo run --release -- <this directory>`. Samples are quantised to 16 bit as `round(x * 32767)`; the
decoders read them back as `n / 32768`, which is what `reference(.., 16)` in the tests computes, so FLAC and ALAC
must round-trip bit-exactly.

## SHA-256

```
b21a3448c06f15c2fb437005410265696ecbd84683a9304e20f981d5aac97090  sweep_alac_a.m4a
55dacc795975df2c2fa4af1d31dd8cb3cc4b2037870aa762f0467bf7668da0bb  sweep_alac_b.m4a
3b5093274e1d48e3333b330cb1ebd71c266f9793cae41bed7f05c2c12b548f2c  sweep_flac16_a.flac
3d74e2b8669e8a7da5ca8a18adc7768fcd43e14467932da64d9180660de9cf15  sweep_flac16_b.flac
a748134ee109a694c4d2d5218b1fa4c86b9f9cdf0bacebe9809941924f0d55f7  sweep_mp3lame_a.mp3
4596d8374e49c36bce736c3cc63f6a081cd3dd24539876b47206974f45a2a935  sweep_mp3lame_b.mp3
```

Do not regenerate casually: the tests pin sample-exact behaviour against these files. If they must change, change the
parameters above and the constants in `tests/common/mod.rs` together.
