use crate::QueueItemId;

/// Everything that can go wrong in the engine. `Clone` so it can travel inside events.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlaybackError {
    /// The source could not be opened or read.
    #[error("cannot open source: {0}")]
    Open(String),
    /// No demuxer recognised the file.
    #[error("unrecognised or damaged media format: {0}")]
    Probe(String),
    /// The container has no audio track.
    #[error("no audio track in file")]
    NoAudioTrack,
    /// No decoder for the track's codec, or its parameters are incomplete.
    #[error("unsupported codec: {0}")]
    UnsupportedCodec(String),
    /// The track's rate cannot be converted to the stream rate (a zero rate, or a pair of rates
    /// with so small a common divisor that the resampler would need a block about a second long).
    #[error("track sample rate {track_rate} Hz differs from output rate {output_rate} Hz")]
    SampleRateMismatch {
        /// The file's rate.
        track_rate: u32,
        /// The output stream's rate.
        output_rate: u32,
    },
    /// Down-mixing is not part of the engine core.
    #[error("{channels} channel audio is not supported (mono and stereo only)")]
    UnsupportedChannels {
        /// The file's channel count.
        channels: usize,
    },
    /// The decoder or demuxer failed mid-stream.
    #[error("decode failed: {0}")]
    Decode(String),
    /// Seeking failed inside the container.
    #[error("seek failed: {0}")]
    Seek(String),
    /// The requested frame is behind the end of the track.
    #[error("seek target frame {requested} is beyond the track end")]
    SeekOutOfRange {
        /// The requested frame.
        requested: u64,
    },
    /// The output backend failed.
    #[error("audio output: {0}")]
    Output(String),
    /// An `OpenRequest::exclusive_only` open that the device would not take: the rate or format
    /// is not available exclusively, or another application holds the endpoint. The engine
    /// treats exactly this error as a refusal of the rate (it opens again at its own rate); every
    /// other output error is a failure of the device.
    #[error("exclusive mode refused: {0}")]
    ExclusiveRefused(String),
    /// A call argument is invalid.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// A seek or other command named a queue item the engine does not have.
    #[error("queue item {0:?} is not in the queue")]
    UnknownItem(QueueItemId),
    /// A queue edit was built on a view of the queue that is out of date: the item it names as the
    /// current one is not the one the engine is on, so nothing was changed.
    #[error("the queue moved on: {expected:?} is not the current item (it is {current:?})")]
    StaleQueue {
        /// The item the caller saw as current.
        expected: QueueItemId,
        /// The item the engine is on, if it is on any.
        current: Option<QueueItemId>,
    },
    /// The command channel to the decoder thread is full; try again.
    #[error("decoder thread is busy")]
    Busy,
    /// The decoder thread has exited.
    #[error("playback engine has stopped")]
    EngineStopped,
    /// Nothing is loaded.
    #[error("nothing is playing")]
    NothingLoaded,
}
