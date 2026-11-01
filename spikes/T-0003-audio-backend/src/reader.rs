//! Thin Symphonia wrapper: open, sample-accurate seek, pull interleaved stereo f32.
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

pub struct TrackReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    pub rate: u32,
    pub src_channels: usize,
    /// Playable frames (excludes delay/padding) when the container says so.
    pub total_frames: Option<u64>,
    pub delay: Option<u32>,
    pub padding: Option<u32>,
    pub codec: String,
    skip: u64,
    /// Container-independent gapless override (iTunSMPB / edit list parsed by the library layer).
    delay_override: u64,
    valid_override: Option<u64>,
    /// Position (frames, in the trimmed track timeline) of the next frame `next_chunk` will emit.
    pub pos: u64,
    raw: Vec<f32>,
}

impl TrackReader {
    pub fn open(path: &Path, gapless: bool) -> Result<Self> {
        let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| anyhow!("probe: {e}"))?;
        let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow!("no audio track"))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| anyhow!("no audio params"))?;
        let mut opts = AudioDecoderOptions::default();
        opts.gapless = gapless;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &opts)
            .map_err(|e| anyhow!("decoder: {e}"))?;
        let rate = params.sample_rate.ok_or_else(|| anyhow!("no sample rate"))?;
        let src_channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2);
        let codec = format!("{:?}", params.codec);
        let (total_frames, delay, padding) = (track.num_frames, track.delay, track.padding);
        let track_id = track.id;
        Ok(Self {
            format,
            decoder,
            track_id,
            rate,
            src_channels,
            total_frames,
            delay,
            padding,
            codec,
            skip: 0,
            delay_override: 0,
            valid_override: None,
            pos: 0,
            raw: Vec::new(),
        })
    }

    /// Apply external gapless info: drop `delay` leading frames and stop after `valid` frames.
    pub fn set_gapless_override(&mut self, delay: u64, valid: u64) {
        self.delay_override = delay;
        self.valid_override = Some(valid);
        self.skip = delay;
    }

    /// Seek to an absolute frame in the trimmed timeline. Accurate: decodes and discards up to the target.
    pub fn seek(&mut self, frame: u64) -> Result<()> {
        let frame_user = frame;
        let frame = frame + self.delay_override;
        let time = Time::from_nanos((frame as u128 * 1_000_000_000u128 / self.rate as u128) as i64);
        let seeked = self
            .format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(|e| anyhow!("seek: {e}"))?;
        self.decoder.reset();
        let tb = self
            .format
            .tracks()
            .iter()
            .find(|t| t.id == self.track_id)
            .and_then(|t| t.time_base)
            .ok_or_else(|| anyhow!("no timebase"))?;
        let to_frames = |ts: symphonia::core::units::Timestamp| -> i64 {
            let t = tb.calc_time_saturating(ts);
            (t.as_secs_f64() * self.rate as f64).round() as i64
        };
        let actual = to_frames(seeked.actual_ts).max(0) as u64;
        self.skip = frame.saturating_sub(actual);
        self.pos = frame_user;
        Ok(())
    }

    /// Decode the next packet and append interleaved STEREO f32 to `out`. Returns false at EOF.
    pub fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(e) => bail!("next_packet: {e}"),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let buf = match self.decoder.decode(&packet) {
                Ok(b) => b,
                Err(Error::DecodeError(_)) | Err(Error::IoError(_)) => continue,
                Err(e) => bail!("decode: {e}"),
            };
            let frames = buf.frames();
            if frames == 0 {
                continue;
            }
            let ch = buf.spec().channels().count();
            self.raw.resize(frames * ch, 0.0);
            buf.copy_to_slice_interleaved::<f32, _>(&mut self.raw[..]);
            let skip = (self.skip as usize).min(frames);
            self.skip -= skip as u64;
            let mut keep = frames - skip;
            if let Some(valid) = self.valid_override {
                if self.pos >= valid {
                    return Ok(false);
                }
                keep = keep.min((valid - self.pos) as usize);
            }
            if keep == 0 {
                continue;
            }
            for f in skip..skip + keep {
                let (l, r) = match ch {
                    1 => (self.raw[f], self.raw[f]),
                    _ => (self.raw[f * ch], self.raw[f * ch + 1]),
                };
                out.push(l);
                out.push(r);
            }
            self.pos += keep as u64;
            return Ok(true);
        }
    }
}
