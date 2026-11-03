use std::path::PathBuf;

/// Identifies one queue edit ([`Engine::replace_after`](crate::Engine::replace_after)), so that the
/// verdict the decoder thread reports later can be matched to the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditId(pub u64);

/// Identifies one entry of the play queue for the lifetime of that entry.
///
/// Assigned by the queue layer above the engine; the engine only echoes it in marks and events.
/// Unique within the queue handed to [`Engine::load`](crate::Engine::load).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueItemId(pub u64);

/// Encoder delay and real length of a file, for formats whose container Symphonia cannot trim
/// itself (M4A/AAC edit lists and `iTunSMPB`, extracted by the library layer, T-0018).
///
/// MP3 with a LAME/Xing tag, FLAC, WAV and ALAC need none: Symphonia trims or they are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaplessInfo {
    /// Leading frames to drop.
    pub delay_frames: u64,
    /// Frames to play after the delay; everything after is padding and is dropped.
    pub valid_frames: u64,
}

/// One playable file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Queue identity.
    pub id: QueueItemId,
    /// Location handed to the engine's [`SourceOpener`](crate::SourceOpener).
    pub path: PathBuf,
    /// External gapless trim, when the library layer knows one.
    pub gapless: Option<GaplessInfo>,
}

/// Where playback is, in frames of the current track's (trimmed) timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The track being heard.
    pub item: QueueItemId,
    /// Frame within the track.
    pub frame: u64,
    /// `frame` in milliseconds at the output sample rate.
    pub position_ms: u64,
}

/// Channels the engine renders. Mono is duplicated, anything beyond stereo is rejected.
pub(crate) const CHANNELS: usize = 2;
