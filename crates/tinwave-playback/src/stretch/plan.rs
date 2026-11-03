//! The rate arithmetic that keeps the source position exact (ADR 0006 decision 3).
//!
//! The stretcher is driven with a **fixed number of output frames per call** and a variable number
//! of input frames: `floor(out * rate + carry)`, the fractional remainder carried to the next
//! call. The ratio of input to output is then exact by construction, whatever the rate, and so is
//! the source position: after `n` calls the stretcher has been given `floor(n * out * rate)`
//! frames (within one frame of the exact product), which is what the engine's position marks and
//! the time-weighted rate rely on.

use super::StretchError;

/// Output frames of each [`Stretcher::process`](super::Stretcher::process) call the engine makes.
/// A fixed size keeps the per-call cost flat; at 44.1 kHz it is about 23 ms.
pub const BLOCK_FRAMES: usize = 1024;

/// Source frames to feed for each fixed-size output block at one playback rate.
///
/// ```
/// use tinwave_playback::stretch::BlockPlan;
/// # fn main() -> Result<(), tinwave_playback::stretch::StretchError> {
/// let mut plan = BlockPlan::new(1.1)?;
/// // 1.1 * 1000 = 1100 (the sliver of a frame left over is carried).
/// assert_eq!(plan.next_input_frames(1000), 1100);
/// // 1.1 * 3 = 3.3: three frames now, the 0.3 carried; four blocks later it adds up to a frame.
/// let fed: usize = (0..4).map(|_| plan.next_input_frames(3)).sum();
/// assert_eq!(fed, 13); // floor(4 * 3.3) = 13, not 4 * 3 = 12
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BlockPlan {
    rate: f64,
    carry: f64,
}

impl BlockPlan {
    /// A plan for `rate` source frames per output frame (1.25 consumes 5 source frames for every
    /// 4 produced), with nothing carried yet.
    ///
    /// # Errors
    /// [`StretchError::BadRate`] unless `rate` is positive and finite.
    pub fn new(rate: f64) -> Result<Self, StretchError> {
        if !(rate.is_finite() && rate > 0.0) {
            return Err(StretchError::BadRate { rate });
        }
        Ok(Self { rate, carry: 0.0 })
    }

    /// The playback rate.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// The fraction of a source frame carried to the next call, in `0.0..1.0`.
    #[must_use]
    pub fn carry(&self) -> f64 {
        self.carry
    }

    /// How many source frames [`next_input_frames`](Self::next_input_frames) would return for
    /// `out_frames`, without committing to it.
    #[must_use]
    pub fn peek_input_frames(&self, out_frames: usize) -> usize {
        // `as` saturates: a count beyond usize::MAX is not a real block.
        self.wanted(out_frames).floor() as usize
    }

    /// Source frames to feed for the next `out_frames` output frames; the fractional remainder is
    /// carried to the next call.
    pub fn next_input_frames(&mut self, out_frames: usize) -> usize {
        let wanted = self.wanted(out_frames);
        let take = wanted.floor();
        self.carry = wanted - take;
        take as usize
    }

    fn wanted(&self, out_frames: usize) -> f64 {
        out_frames as f64 * self.rate + self.carry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_total_input_is_the_exact_product_to_within_one_frame() {
        for rate in [0.75, 1.0, 1.1, 1.25, 1.5, 0.9, 1.3333] {
            for out in [1usize, 7, 441, 1000, BLOCK_FRAMES] {
                let mut plan = BlockPlan::new(rate).expect("plan");
                let mut fed = 0usize;
                for calls in 1..=2000usize {
                    fed += plan.next_input_frames(out);
                    let exact = (calls * out) as f64 * rate;
                    assert!(
                        fed as f64 <= exact + 1e-6 && exact - (fed as f64) < 1.0 + 1e-6,
                        "rate {rate}, {out} out: {fed} fed after {calls} calls, exact {exact}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_carry_stays_a_fraction() {
        let mut plan = BlockPlan::new(1.1).expect("plan");
        for _ in 0..10_000 {
            plan.next_input_frames(333);
            assert!((0.0..1.0).contains(&plan.carry()), "{}", plan.carry());
        }
    }

    #[test]
    fn exact_rates_need_no_carry() {
        for (rate, per_block) in [(0.75, 768usize), (1.0, 1024), (1.25, 1280), (1.5, 1536)] {
            let mut plan = BlockPlan::new(rate).expect("plan");
            for _ in 0..100 {
                assert_eq!(plan.next_input_frames(BLOCK_FRAMES), per_block, "rate {rate}");
                assert_eq!(plan.carry(), 0.0);
            }
        }
    }

    #[test]
    fn peeking_commits_to_nothing() {
        let mut plan = BlockPlan::new(1.1).expect("plan");
        let before = plan.clone();
        let peeked = plan.peek_input_frames(3);
        assert_eq!(plan, before);
        assert_eq!(plan.next_input_frames(3), peeked);
    }

    #[test]
    fn the_carry_decides_which_blocks_take_the_extra_frame() {
        // 0.5 per output frame, 1 frame per call: every second call takes a frame.
        let mut plan = BlockPlan::new(0.5).expect("plan");
        let taken: Vec<usize> = (0..6).map(|_| plan.next_input_frames(1)).collect();
        assert_eq!(taken, [0, 1, 0, 1, 0, 1]);
    }

    #[test]
    fn nonsense_rates_are_refused() {
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(BlockPlan::new(rate), Err(StretchError::BadRate { .. })),
                "rate {rate}"
            );
        }
    }
}
