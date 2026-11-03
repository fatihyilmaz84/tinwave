//! Thin Symphonia wrapper: open, accurate seek, pull interleaved stereo `f32`.
//!
//! Runs on the decoder thread only. Symphonia is used unmodified (ADR 0005, D-0003).

use std::io;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::{Error, SeekErrorKind};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::types::CHANNELS;
use crate::{PlaybackError, Track};

/// Opens the bytes of a track. The decoder thread calls this; reads may block.
///
/// The default [`FsOpener`] reads local files. Server-streamed or downloaded sources plug in
/// here later; tests use it to inject a reader that blocks.
pub trait SourceOpener: Send + Sync + std::fmt::Debug {
    /// Open `track` for reading.
    ///
    /// # Errors
    /// Any IO error; it becomes a [`PlaybackError::Open`] for that track.
    fn open(&self, track: &Track) -> io::Result<Box<dyn MediaSource>>;
}

/// Opens `Track::path` from the local file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsOpener;

impl SourceOpener for FsOpener {
    fn open(&self, track: &Track) -> io::Result<Box<dyn MediaSource>> {
        Ok(Box::new(std::fs::File::open(&track.path)?))
    }
}

/// Frames decoded and dropped before a seek target to warm up codec state (four MP3 frames).
const SEEK_PREROLL_FRAMES: u64 = 4_608;

pub(crate) struct TrackReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    rate: u32,
    /// Frames still to drop before output starts (seek landing, external delay).
    skip: u64,
    /// External delay (see `GaplessInfo`): the trimmed timeline starts this many frames into the
    /// stream.
    delay: u64,
    /// External valid length, if any.
    valid: Option<u64>,
    /// Playable frames of the trimmed timeline when known: the external valid length, else the
    /// container's own count (which already excludes encoder delay and padding).
    length: Option<u64>,
    /// Frame (trimmed timeline) the next emitted frame will have.
    pos: u64,
    /// A seek landed exactly on the end of the track: there is nothing left to read.
    at_end: bool,
    /// Packets skipped because the decoder reported them corrupt.
    corrupt_packets: u64,
    raw: Vec<f32>,
}

impl std::fmt::Debug for TrackReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackReader")
            .field("rate", &self.rate)
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

impl TrackReader {
    pub fn open(opener: &dyn SourceOpener, track: &Track) -> Result<Self, PlaybackError> {
        let source = opener
            .open(track)
            .map_err(|e| PlaybackError::Open(e.to_string()))?;
        let stream = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = track.path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| PlaybackError::Probe(e.to_string()))?;
        let audio_track = format
            .default_track(TrackType::Audio)
            .ok_or(PlaybackError::NoAudioTrack)?;
        let params = audio_track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| PlaybackError::UnsupportedCodec("missing audio parameters".into()))?;
        let rate = params
            .sample_rate
            .ok_or_else(|| PlaybackError::UnsupportedCodec("missing sample rate".into()))?;
        let channels = params.channels.as_ref().map_or(CHANNELS, |c| c.count());
        if channels == 0 || channels > CHANNELS {
            return Err(PlaybackError::UnsupportedChannels { channels });
        }
        // Symphonia trims MP3 encoder delay/padding from the LAME/Xing tag when `gapless` is on.
        let mut options = AudioDecoderOptions::default();
        options.gapless = true;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &options)
            .map_err(|e| PlaybackError::UnsupportedCodec(e.to_string()))?;
        let track_id = audio_track.id;
        let container_frames = audio_track.num_frames;
        let (delay, valid) = track
            .gapless
            .map_or((0, None), |g| (g.delay_frames, Some(g.valid_frames)));
        Ok(Self {
            format,
            decoder,
            track_id,
            rate,
            skip: delay,
            delay,
            valid,
            length: valid.or(container_frames),
            pos: 0,
            at_end: false,
            corrupt_packets: 0,
            raw: Vec::new(),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Playable frames of the trimmed timeline (file frames), when the file says.
    pub fn length(&self) -> Option<u64> {
        self.length
    }

    pub fn take_corrupt_packets(&mut self) -> u64 {
        std::mem::take(&mut self.corrupt_packets)
    }

    /// Position the reader `preroll` frames before `frame`, for a consumer that needs history to
    /// warm up on (the time stretcher). Returns how many frames of history will precede `frame`:
    /// `preroll`, or fewer at the start of the track, or none at its end. Errors as
    /// [`seek`](Self::seek) for a `frame` behind the end, whatever the preroll.
    pub fn seek_with_preroll(&mut self, frame: u64, preroll: u64) -> Result<u64, PlaybackError> {
        if self.length.is_some_and(|length| frame > length) {
            return Err(PlaybackError::SeekOutOfRange { requested: frame });
        }
        let preroll = if self.length == Some(frame) {
            0
        } else {
            preroll.min(frame)
        };
        // A reader that was just opened is at frame 0 already.
        if frame > preroll {
            self.seek(frame - preroll)?;
        }
        Ok(preroll)
    }

    /// Seek to `frame` of the trimmed timeline. Lands sample-exact: the container seeks to a packet
    /// at or before the target and the remainder is decoded and dropped.
    ///
    /// `frame == length` is the end of the track, not an error: the reader is positioned there and
    /// the next [`next_chunk`](Self::next_chunk) reports the end, so the track counts as finished
    /// and the queue moves on (this is also where a device switch lands when it happens to coincide
    /// with a track boundary). Only `frame > length` is out of range.
    pub fn seek(&mut self, frame: u64) -> Result<(), PlaybackError> {
        // Detected here instead of trusting the demuxer to report it: some clamp to the last packet.
        if self.length.is_some_and(|length| frame > length) {
            return Err(PlaybackError::SeekOutOfRange { requested: frame });
        }
        self.at_end = self.length == Some(frame);
        if self.at_end {
            self.pos = frame;
            return Ok(());
        }
        let stream_frame = frame + self.delay;
        // Lossy codecs carry state across packets (MP3 bit reservoir and overlap-add, AAC and
        // Vorbis overlap). Landing exactly on the target packet would play its first few hundred
        // frames wrong, so land a little earlier and decode-and-drop up to the target.
        let landing = stream_frame.saturating_sub(SEEK_PREROLL_FRAMES);
        let nanos = u128::from(landing) * 1_000_000_000 / u128::from(self.rate);
        let time = Time::from_nanos(i64::try_from(nanos).unwrap_or(i64::MAX));
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| match e {
                Error::SeekError(SeekErrorKind::OutOfRange) => {
                    PlaybackError::SeekOutOfRange { requested: frame }
                }
                other => PlaybackError::Seek(other.to_string()),
            })?;
        self.decoder.reset();
        let time_base = self
            .format
            .tracks()
            .iter()
            .find(|t| t.id == self.track_id)
            .and_then(|t| t.time_base)
            .ok_or_else(|| PlaybackError::Seek("track has no time base".into()))?;
        let actual_secs = time_base
            .calc_time_saturating(seeked.actual_ts)
            .as_secs_f64();
        // Rounded: the container timestamp is exact in frames, only the f64 hop adds noise.
        let landed = (actual_secs * f64::from(self.rate)).round().max(0.0) as u64;
        if landed > stream_frame {
            return Err(PlaybackError::Seek(format!(
                "container landed at frame {landed}, after the target {stream_frame}"
            )));
        }
        self.skip = stream_frame - landed;
        self.pos = frame;
        Ok(())
    }

    /// Decode the next packet and append interleaved stereo `f32` to `out`.
    /// `Ok(false)` at the end of the track.
    pub fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, PlaybackError> {
        loop {
            if self.at_end || self.valid.is_some_and(|valid| self.pos >= valid) {
                return Ok(false);
            }
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                Err(Error::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(false)
                }
                Err(e) => return Err(PlaybackError::Decode(e.to_string())),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // A corrupt packet is skipped and counted; the caller reports the count.
                Err(Error::DecodeError(_)) => {
                    self.corrupt_packets += 1;
                    continue;
                }
                Err(e) => return Err(PlaybackError::Decode(e.to_string())),
            };
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let channels = decoded.spec().channels().count();
            if channels == 0 || channels > CHANNELS {
                return Err(PlaybackError::UnsupportedChannels { channels });
            }
            self.raw.resize(frames * channels, 0.0);
            decoded.copy_to_slice_interleaved::<f32, _>(&mut self.raw[..]);
            let skip = usize::try_from(self.skip).unwrap_or(usize::MAX).min(frames);
            self.skip -= skip as u64;
            let mut keep = frames - skip;
            if let Some(valid) = self.valid {
                keep = keep.min(usize::try_from(valid - self.pos).unwrap_or(usize::MAX));
            }
            if keep == 0 {
                continue;
            }
            let wanted = &self.raw[skip * channels..(skip + keep) * channels];
            if channels == 1 {
                out.extend(wanted.iter().flat_map(|&s| [s, s]));
            } else {
                out.extend_from_slice(wanted);
            }
            self.pos += keep as u64;
            return Ok(true);
        }
    }
}
