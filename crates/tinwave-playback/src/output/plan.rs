//! Choosing an exclusive-mode format. Pure logic: the caller supplies the device's answers.

use super::convert::SampleFormat;
use super::info::StreamMode;

/// Integer formats tried for exclusive mode, in order of preference. The spike (ADR 0005) found
/// endpoints that accept i16, i24-in-32 and i32 but not f32 or packed 24-bit.
pub const EXCLUSIVE_CANDIDATES: [SampleFormat; 3] =
    [SampleFormat::I24In32, SampleFormat::I32, SampleFormat::I16];

/// The first candidate the device accepts, or the reason none was: every refusal, in order, so
/// the fallback to shared mode can be shown to the user.
///
/// # Errors
/// A human-readable list of what the device said for each format.
pub fn plan_exclusive(
    mut ask: impl FnMut(SampleFormat) -> Result<(), String>,
) -> Result<SampleFormat, String> {
    let mut refusals = Vec::new();
    for format in EXCLUSIVE_CANDIDATES {
        match ask(format) {
            Ok(()) => return Ok(format),
            Err(why) => refusals.push(format!("{format:?}: {why}")),
        }
    }
    Err(format!(
        "no integer format accepted for exclusive mode ({})",
        refusals.join("; ")
    ))
}

/// How a stream ended up being opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeOutcome {
    /// The mode in use.
    pub mode: StreamMode,
    /// The format written to the device (`F32` in shared mode).
    pub format: SampleFormat,
    /// The device period in 100 ns units: the exclusive period, or the shared default.
    pub period_hns: i64,
    /// Why exclusive mode was not used although it was asked for. Never silent.
    pub fallback: Option<String>,
}

/// Exclusive when asked for and the device allows it; otherwise shared, with the reason.
///
/// `try_exclusive` probes and opens the exclusive stream and returns its format and period, or
/// the reason it could not. It is not called when exclusive mode was not asked for.
pub fn resolve_mode(
    want_exclusive: bool,
    try_exclusive: impl FnOnce() -> Result<(SampleFormat, i64), String>,
    shared_period_hns: i64,
) -> ModeOutcome {
    if want_exclusive {
        match try_exclusive() {
            Ok((format, period_hns)) => {
                return ModeOutcome {
                    mode: StreamMode::Exclusive,
                    format,
                    period_hns,
                    fallback: None,
                }
            }
            Err(why) => {
                return ModeOutcome {
                    mode: StreamMode::Shared,
                    format: SampleFormat::F32,
                    period_hns: shared_period_hns,
                    fallback: Some(format!("exclusive mode refused: {why}")),
                }
            }
        }
    }
    ModeOutcome {
        mode: StreamMode::Shared,
        format: SampleFormat::F32,
        period_hns: shared_period_hns,
        fallback: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_is_the_default_and_never_probes_the_device() {
        let outcome = resolve_mode(false, || panic!("must not probe"), 100_000);
        assert_eq!(outcome.mode, StreamMode::Shared);
        assert_eq!(outcome.format, SampleFormat::F32);
        assert_eq!(outcome.fallback, None);
    }

    #[test]
    fn exclusive_success_reports_format_and_period() {
        let outcome = resolve_mode(true, || Ok((SampleFormat::I24In32, 40_000)), 100_000);
        assert_eq!(outcome.mode, StreamMode::Exclusive);
        assert_eq!(outcome.format, SampleFormat::I24In32);
        assert_eq!(outcome.period_hns, 40_000);
        assert_eq!(outcome.fallback, None);
    }

    #[test]
    fn a_refused_exclusive_request_falls_back_to_shared_with_a_visible_reason() {
        let outcome = resolve_mode(
            true,
            || Err("the endpoint is in use by another exclusive client".into()),
            100_000,
        );
        assert_eq!(outcome.mode, StreamMode::Shared);
        assert_eq!(outcome.format, SampleFormat::F32);
        assert_eq!(outcome.period_hns, 100_000);
        let reason = outcome.fallback.expect("a reason");
        assert!(reason.starts_with("exclusive mode refused"), "{reason}");
        assert!(reason.contains("another exclusive client"), "{reason}");
    }

    #[test]
    fn a_device_that_refuses_every_format_gives_the_formats_in_the_fallback_reason() {
        let outcome = resolve_mode(
            true,
            || plan_exclusive(|_| Err("unsupported".into())).map(|f| (f, 0)),
            100_000,
        );
        let reason = outcome.fallback.expect("a reason");
        assert!(
            reason.contains("I24In32") && reason.contains("I16"),
            "{reason}"
        );
    }

    #[test]
    fn prefers_24_in_32_then_32_then_16() {
        assert_eq!(plan_exclusive(|_| Ok(())), Ok(SampleFormat::I24In32));
        assert_eq!(
            plan_exclusive(|f| (f != SampleFormat::I24In32)
                .then_some(())
                .ok_or_else(|| "no".into())),
            Ok(SampleFormat::I32)
        );
        assert_eq!(
            plan_exclusive(|f| (f == SampleFormat::I16)
                .then_some(())
                .ok_or_else(|| "no".into())),
            Ok(SampleFormat::I16)
        );
    }

    #[test]
    fn a_device_that_refuses_everything_gives_a_reason_naming_each_format() {
        let reason = plan_exclusive(|_| Err("not supported".into())).expect_err("refused");
        for name in ["I24In32", "I32", "I16"] {
            assert!(reason.contains(name), "{reason}");
        }
        assert!(reason.contains("not supported"));
    }

    #[test]
    fn f32_is_never_offered_for_exclusive_mode() {
        assert!(!EXCLUSIVE_CANDIDATES.contains(&SampleFormat::F32));
    }
}
