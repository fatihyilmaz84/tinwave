//! Test-signal generation: a log sine sweep split in two files (A | B) at a deliberately
//! codec-frame-unaligned boundary, in every format we can produce.
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub const AMP: f64 = 0.1; // -20 dBFS

/// Continuous exponential sweep f0..f1 over `total` frames. L = sweep, R = 0.5 * sweep.
pub fn sweep(rate: u32, total: usize, f0: f64, f1: f64) -> (Vec<f64>, Vec<f64>) {
    let t_total = total as f64 / rate as f64;
    let k = (f1 / f0).ln();
    let mut l = Vec::with_capacity(total);
    let mut r = Vec::with_capacity(total);
    for n in 0..total {
        let t = n as f64 / rate as f64;
        let phase = 2.0 * std::f64::consts::PI * f0 * t_total / k * ((t / t_total * k).exp() - 1.0);
        let s = AMP * phase.sin();
        l.push(s);
        r.push(0.5 * s);
    }
    (l, r)
}

pub fn to_i16(x: &[f64]) -> Vec<i16> {
    x.iter().map(|v| (v * 32767.0).round() as i16).collect()
}
pub fn to_i24(x: &[f64]) -> Vec<i32> {
    x.iter().map(|v| (v * 8388607.0).round() as i32).collect()
}

fn interleave<T: Copy>(l: &[T], r: &[T]) -> Vec<T> {
    let mut o = Vec::with_capacity(l.len() * 2);
    for i in 0..l.len() {
        o.push(l[i]);
        o.push(r[i]);
    }
    o
}

pub fn write_wav16(p: &Path, rate: u32, l: &[f64], r: &[f64]) -> Result<()> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(p, spec)?;
    for s in interleave(&to_i16(l), &to_i16(r)) {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}
pub fn write_wav24(p: &Path, rate: u32, l: &[f64], r: &[f64]) -> Result<()> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 24, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(p, spec)?;
    for s in interleave(&to_i24(l), &to_i24(r)) {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}
pub fn write_wavf32(p: &Path, rate: u32, l: &[f64], r: &[f64]) -> Result<()> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(p, spec)?;
    for i in 0..l.len() {
        w.write_sample(l[i] as f32)?;
        w.write_sample(r[i] as f32)?;
    }
    w.finalize()?;
    Ok(())
}

pub fn write_flac(p: &Path, rate: u32, bits: usize, l: &[f64], r: &[f64]) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let samples: Vec<i32> = if bits == 16 {
        interleave(&to_i16(l), &to_i16(r)).into_iter().map(|v| v as i32).collect()
    } else {
        interleave(&to_i24(l), &to_i24(r))
    };
    let cfg = flacenc::config::Encoder::default().into_verified().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let src = flacenc::source::MemSource::from_samples(&samples, 2, bits, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&cfg, src, cfg.block_size).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    // flacenc records the short LAST block as min_blocksize; the spec says min excludes the last block
    // (libFLAC does it that way). Patch STREAMINFO so the stream looks like libFLAC output.
    let mut bytes = sink.as_slice().to_vec();
    let (maxb0, maxb1) = (bytes[10], bytes[11]);
    bytes[8] = maxb0;
    bytes[9] = maxb1;
    std::fs::write(p, bytes)?;
    Ok(())
}

/// MP3 via LAME (LGPL, test-generation only). `lame_tag` controls the Xing/Info + LAME gapless header.
pub fn write_mp3(p: &Path, rate: u32, l: &[f64], r: &[f64], lame_tag: bool) -> Result<()> {
    use mp3lame_encoder::*;
    let mut b = Builder::new().ok_or_else(|| anyhow::anyhow!("lame init"))?;
    b.set_num_channels(2).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    b.set_sample_rate(rate).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    b.set_brate(Bitrate::Kbps192).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    b.set_quality(Quality::Good).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    b.set_to_write_vbr_tag(lame_tag).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut enc = b.build().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let (li, ri) = (to_i16(l), to_i16(r));
    let input = DualPcm { left: &li, right: &ri };
    let mut out: Vec<u8> = Vec::new();
    out.reserve(max_required_buffer_size(li.len()));
    let n = enc.encode(input, out.spare_capacity_mut()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    unsafe { out.set_len(out.len() + n) };
    out.reserve(8192);
    let n = enc.flush::<FlushGap>(out.spare_capacity_mut()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    unsafe { out.set_len(out.len() + n) };
    if lame_tag && enc.lame_tag_size() > 0 {
        let mut tag = Vec::with_capacity(enc.lame_tag_size());
        enc.lame_tag_encode_to_vec(&mut tag);
        // LAME reserves the first frame for the tag; overwrite it in place.
        if tag.len() > out.len() {
            bail!("tag larger than stream");
        }
        out[..tag.len()].copy_from_slice(&tag);
    }
    std::fs::write(p, out)?;
    Ok(())
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}
fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + body.len());
    v.extend(be32((8 + body.len()) as u32));
    v.extend(kind);
    v.extend(body);
    v
}
fn fullatom(kind: &[u8; 4], ver_flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = be32(ver_flags).to_vec();
    b.extend(body);
    atom(kind, &b)
}

/// Minimal MP4 muxer for ALAC: ftyp, moov(trak..), mdat. 16-bit stereo.
pub fn write_alac_m4a(p: &Path, rate: u32, l: &[f64], r: &[f64]) -> Result<()> {
    use alac_encoder::{AlacEncoder, FormatDescription};
    const FRAME: u32 = 4096;
    let inf = FormatDescription::pcm::<i16>(rate as f64, 2);
    let outf = FormatDescription::alac(rate as f64, FRAME, 2);
    let mut enc = AlacEncoder::new(&outf);
    let pcm = interleave(&to_i16(l), &to_i16(r));
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut buf = vec![0u8; outf.max_packet_size()];
    let mut packets: Vec<Vec<u8>> = vec![];
    let mut durs: Vec<u32> = vec![];
    for chunk in bytes.chunks(FRAME as usize * 4) {
        let n = enc.encode(&inf, chunk, &mut buf);
        packets.push(buf[..n].to_vec());
        durs.push((chunk.len() / 4) as u32);
    }
    let cookie = enc.magic_cookie();
    let total_frames: u64 = durs.iter().map(|d| *d as u64).sum();

    let ftyp = {
        let mut b = b"M4A ".to_vec();
        b.extend(be32(0));
        b.extend(b"M4A mp42isom");
        atom(b"ftyp", &b)
    };
    let build_moov = |mdat_data_off: u32| -> Vec<u8> {
        let mut mvhd = vec![0u8; 8]; // ctime, mtime
        mvhd.extend(be32(rate));
        mvhd.extend(be32(total_frames as u32));
        mvhd.extend(be32(0x0001_0000)); // rate 1.0
        mvhd.extend([0x01, 0x00, 0, 0]); // volume 1.0 + reserved
        mvhd.extend([0u8; 8]);
        for v in [0x10000u32, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000] {
            mvhd.extend(be32(v));
        }
        mvhd.extend([0u8; 24]);
        mvhd.extend(be32(2)); // next track id
        let mvhd = fullatom(b"mvhd", 0, &mvhd);

        let mut tkhd = vec![0u8; 8];
        tkhd.extend(be32(1));
        tkhd.extend(be32(0));
        tkhd.extend(be32(total_frames as u32));
        tkhd.extend([0u8; 8]);
        tkhd.extend([0u8; 4]); // layer, alt group
        tkhd.extend([0x01, 0x00, 0, 0]); // volume
        for v in [0x10000u32, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000] {
            tkhd.extend(be32(v));
        }
        tkhd.extend([0u8; 8]);
        let tkhd = fullatom(b"tkhd", 3, &tkhd);

        let mut mdhd = vec![0u8; 8];
        mdhd.extend(be32(rate));
        mdhd.extend(be32(total_frames as u32));
        mdhd.extend([0x55, 0xC4, 0, 0]);
        let mdhd = fullatom(b"mdhd", 0, &mdhd);
        let mut hdlr = vec![0u8; 4];
        hdlr.extend(b"soun");
        hdlr.extend([0u8; 12]);
        hdlr.extend(b"SoundHandler\0");
        let hdlr = fullatom(b"hdlr", 0, &hdlr);

        let smhd = fullatom(b"smhd", 0, &[0u8; 4]);
        let url = fullatom(b"url ", 1, &[]);
        let dref = fullatom(b"dref", 0, &[&be32(1)[..], &url].concat());
        let dinf = atom(b"dinf", &dref);

        // sample entry 'alac'
        let inner = fullatom(b"alac", 0, &cookie);
        let mut entry = vec![0u8; 6];
        entry.extend([0, 1]); // data ref idx
        entry.extend([0u8; 8]); // version, revision, vendor
        entry.extend([0, 2, 0, 16]); // channels, sample size
        entry.extend([0u8; 4]); // compression id, packet size
        entry.extend(be32(rate << 16));
        entry.extend(&inner);
        let entry = atom(b"alac", &entry);
        let stsd = fullatom(b"stsd", 0, &[&be32(1)[..], &entry].concat());

        let mut runs: Vec<(u32, u32)> = vec![];
        for d in &durs {
            match runs.last_mut() {
                Some((c, dd)) if *dd == *d => *c += 1,
                _ => runs.push((1, *d)),
            }
        }
        let mut stts = be32(runs.len() as u32).to_vec();
        for (c, d) in &runs {
            stts.extend(be32(*c));
            stts.extend(be32(*d));
        }
        let stts = fullatom(b"stts", 0, &stts);
        let stsc = fullatom(b"stsc", 0, &[be32(1), be32(1), be32(packets.len() as u32), be32(1)].concat());
        let mut stsz = be32(0).to_vec();
        stsz.extend(be32(packets.len() as u32));
        for pk in &packets {
            stsz.extend(be32(pk.len() as u32));
        }
        let stsz = fullatom(b"stsz", 0, &stsz);
        let stco = fullatom(b"stco", 0, &[be32(1), be32(mdat_data_off)].concat());
        let stbl = atom(b"stbl", &[stsd, stts, stsc, stsz, stco].concat());
        let minf = atom(b"minf", &[smhd, dinf, stbl].concat());
        let mdia = atom(b"mdia", &[mdhd, hdlr, minf].concat());
        let trak = atom(b"trak", &[tkhd, mdia].concat());
        atom(b"moov", &[mvhd, trak].concat())
    };
    let moov_len = build_moov(0).len();
    let off = (ftyp.len() + moov_len + 8) as u32;
    let moov = build_moov(off);
    let mdat_body: Vec<u8> = packets.concat();
    let mut f = ftyp;
    f.extend(moov);
    f.extend(atom(b"mdat", &mdat_body));
    std::fs::write(p, f)?;
    Ok(())
}

pub struct GenSet {
    pub rate: u32,
    pub dir: PathBuf,
    pub split: usize,
    pub total: usize,
}

pub fn generate(out: &Path, rate: u32, seconds: f64, split_frames: usize) -> Result<GenSet> {
    std::fs::create_dir_all(out)?;
    let total = (rate as f64 * seconds) as usize;
    let (l, r) = sweep(rate, total, 200.0, 12000.0);
    let (la, lb) = l.split_at(split_frames);
    let (ra, rb) = r.split_at(split_frames);
    let sfx = |name: &str, part: &str| out.join(format!("{name}_{rate}_{part}"));
    for (part, ll, rr) in [("a", la, ra), ("b", lb, rb)] {
        write_wav16(&sfx("wav16", &format!("{part}.wav")), rate, ll, rr)?;
        write_wav24(&sfx("wav24", &format!("{part}.wav")), rate, ll, rr)?;
        write_wavf32(&sfx("wavf32", &format!("{part}.wav")), rate, ll, rr)?;
        write_flac(&sfx("flac16", &format!("{part}.flac")), rate, 16, ll, rr)?;
        write_flac(&sfx("flac24", &format!("{part}.flac")), rate, 24, ll, rr)?;
        write_mp3(&sfx("mp3lame", &format!("{part}.mp3")), rate, ll, rr, true)?;
        write_mp3(&sfx("mp3notag", &format!("{part}.mp3")), rate, ll, rr, false)?;
        write_alac_m4a(&sfx("alac", &format!("{part}.m4a")), rate, ll, rr)?;
        crate::mf_aac::write_aac_m4a(&sfx("aac", &format!("{part}.m4a")), rate, ll, rr)?;
    }
    // Reference (lossless, float) of the whole sweep for comparisons.
    write_wavf32(&out.join(format!("ref_{rate}_full.wav")), rate, &l, &r)?;
    Ok(GenSet { rate, dir: out.to_path_buf(), split: split_frames, total })
}
