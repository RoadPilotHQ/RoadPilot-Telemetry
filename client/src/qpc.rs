//! Frame timing across processes from QueryPerformanceCounter (QPC) stamps.
//!
//! The plugin stamps every schema 2 frame with two QPC values and its QPC frequency:
//!
//! * `producedQpc`: when the game finished the frame (`FRAME_END`, game thread);
//! * `sentQpc`: immediately before the plugin's worker serialized and wrote it;
//! * `qpcFrequency`: ticks per second.
//!
//! The reader adds `received_qpc` ([`crate::ReceivedTelemetryFrame::received_qpc`]) at the
//! same moment it takes `received_at`: after the frame was read and decoded, when it is
//! handed on. QPC is one system-wide monotonic counter on Windows, so values from the game
//! process and from RoadPilot's process are directly comparable on the same machine. The
//! client's cross-process test (`tests/qpc_cross_process.rs`) verifies that on the machine it
//! runs on.
//!
//! [`evaluate`] turns the stamps into durations, and only if they form one consistent,
//! monotonic timeline:
//!
//! * every stamp is present, and the plugin's frequency equals this process's frequency
//!   (both read the same counter, so any difference means the values are not comparable);
//! * `produced <= sent <= received` (and `produced <= now` for [`age_at`]);
//! * the produced-to-received span is at most [`MAX_PLAUSIBLE_SPAN`].
//!
//! Anything else is a [`TimingError`], never a duration: a negative, backwards or implausible
//! difference can therefore never be mistaken for a valid low latency. A frame without valid
//! timing may still be displayed for dry-run diagnostics; consumers that make control
//! decisions must treat it as "age unknown".
//!
//! Conversion: `ticks / frequency` seconds, computed exactly in integers (whole seconds and
//! the remainder in nanoseconds, truncated towards zero). It cannot overflow for any `u64`.

use std::time::Duration;

/// Longest produced-to-received (or produced-to-now) span accepted as a measurement. A longer
/// span means corrupt stamps or values from another boot, not a slow frame: any real frame
/// that old is far beyond every freshness limit anyway.
pub const MAX_PLAUSIBLE_SPAN: Duration = Duration::from_secs(60);

/// Which stamp an error refers to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stamp {
    Produced,
    Sent,
    Received,
    PluginFrequency,
    LocalFrequency,
    Now,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingError {
    /// A stamp is absent: an older plugin, a platform without QPC, or a value the decoder
    /// removed as invalid (`dropped_fields`).
    Missing(Stamp),
    /// The plugin's frequency differs from this process's: the values are not comparable.
    FrequencyMismatch { plugin: u64, local: u64 },
    /// `earlier` is later than `later`: not one monotonic timeline.
    NotMonotonic { earlier: Stamp, later: Stamp },
    /// The span exceeds [`MAX_PLAUSIBLE_SPAN`].
    Implausible { span: Duration },
}

/// The raw stamps of one frame. `None` = absent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stamps {
    pub produced: Option<u64>,
    pub sent: Option<u64>,
    pub received: Option<u64>,
    pub plugin_frequency: Option<u64>,
}

/// Durations of one frame, all from one validated timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTiming {
    /// `FRAME_END` to the plugin's send: waiting for the worker, its 60 Hz pacing and a
    /// previous write that blocked.
    pub plugin_processing: Duration,
    /// Plugin send to client receive: serialization, the write, the pipe, and the client's
    /// read and decode.
    pub transport: Duration,
    /// `FRAME_END` to client receive: the frame's absolute age when it was received.
    pub age_at_receive: Duration,
}

/// `ticks` at `frequency` ticks per second, exactly (truncated to whole nanoseconds).
/// `None` for a zero frequency.
pub fn ticks_to_duration(ticks: u64, frequency: u64) -> Option<Duration> {
    if frequency == 0 {
        return None;
    }
    let seconds = ticks / frequency;
    let remainder = ticks % frequency; // < frequency, so remainder * 1e9 fits in u128
    let nanos = (u128::from(remainder) * 1_000_000_000) / u128::from(frequency);
    Some(Duration::new(seconds, nanos as u32))
}

fn frequency_of(plugin: Option<u64>, local: Option<u64>) -> Result<u64, TimingError> {
    let local = local
        .filter(|f| *f > 0)
        .ok_or(TimingError::Missing(Stamp::LocalFrequency))?;
    let plugin = plugin
        .filter(|f| *f > 0)
        .ok_or(TimingError::Missing(Stamp::PluginFrequency))?;
    if plugin != local {
        return Err(TimingError::FrequencyMismatch { plugin, local });
    }
    Ok(local)
}

/// `later - earlier` as a duration, or an error if `earlier` is after `later`.
fn span(
    earlier: (u64, Stamp),
    later: (u64, Stamp),
    frequency: u64,
) -> Result<Duration, TimingError> {
    let ticks = later
        .0
        .checked_sub(earlier.0)
        .ok_or(TimingError::NotMonotonic {
            earlier: earlier.1,
            later: later.1,
        })?;
    // frequency > 0 was checked by the caller.
    Ok(ticks_to_duration(ticks, frequency).unwrap_or(Duration::MAX))
}

fn plausible(span: Duration) -> Result<Duration, TimingError> {
    if span > MAX_PLAUSIBLE_SPAN {
        Err(TimingError::Implausible { span })
    } else {
        Ok(span)
    }
}

/// Validates the stamps of one frame and computes its durations. `local_frequency` is this
/// process's QPC frequency ([`frequency`]).
pub fn evaluate(stamps: Stamps, local_frequency: Option<u64>) -> Result<FrameTiming, TimingError> {
    let produced = stamps
        .produced
        .ok_or(TimingError::Missing(Stamp::Produced))?;
    let sent = stamps.sent.ok_or(TimingError::Missing(Stamp::Sent))?;
    let received = stamps
        .received
        .ok_or(TimingError::Missing(Stamp::Received))?;
    let frequency = frequency_of(stamps.plugin_frequency, local_frequency)?;
    let plugin_processing = span((produced, Stamp::Produced), (sent, Stamp::Sent), frequency)?;
    let transport = span((sent, Stamp::Sent), (received, Stamp::Received), frequency)?;
    let age_at_receive = plausible(span(
        (produced, Stamp::Produced),
        (received, Stamp::Received),
        frequency,
    )?)?;
    Ok(FrameTiming {
        plugin_processing,
        transport,
        age_at_receive,
    })
}

/// The absolute age at QPC time `now` of a frame produced at `produced`, with the same
/// validation as [`evaluate`].
pub fn age_at(
    produced: Option<u64>,
    plugin_frequency: Option<u64>,
    now: Option<u64>,
    local_frequency: Option<u64>,
) -> Result<Duration, TimingError> {
    let produced = produced.ok_or(TimingError::Missing(Stamp::Produced))?;
    let now = now.ok_or(TimingError::Missing(Stamp::Now))?;
    let frequency = frequency_of(plugin_frequency, local_frequency)?;
    plausible(span(
        (produced, Stamp::Produced),
        (now, Stamp::Now),
        frequency,
    )?)
}

/// The current QPC value, or `None` where QPC is unavailable.
#[cfg(windows)]
pub fn now() -> Option<u64> {
    let mut counter: i64 = 0;
    // SAFETY: writes one i64 through a valid pointer; cannot fail on Windows XP and later.
    let ok =
        unsafe { windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut counter) };
    (ok != 0)
        .then_some(counter)
        .and_then(|c| u64::try_from(c).ok())
}

/// This process's QPC frequency in ticks per second, or `None` where QPC is unavailable. It
/// is fixed at boot, so it is read once.
#[cfg(windows)]
pub fn frequency() -> Option<u64> {
    static FREQUENCY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        let mut frequency: i64 = 0;
        // SAFETY: writes one i64 through a valid pointer.
        let ok = unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
        };
        (ok != 0)
            .then_some(frequency)
            .and_then(|f| u64::try_from(f).ok())
            .filter(|f| *f > 0)
    })
}

#[cfg(not(windows))]
pub fn now() -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub fn frequency() -> Option<u64> {
    None
}

impl crate::ReceivedTelemetryFrame {
    /// The frame's stamps.
    pub fn stamps(&self) -> Stamps {
        Stamps {
            produced: self.frame.produced_qpc,
            sent: self.frame.sent_qpc,
            received: self.received_qpc,
            plugin_frequency: self.frame.qpc_frequency,
        }
    }

    /// The frame's durations against this process's QPC frequency.
    pub fn timing(&self) -> Result<FrameTiming, TimingError> {
        evaluate(self.stamps(), frequency())
    }

    /// The frame's absolute age now.
    pub fn age_now(&self) -> Result<Duration, TimingError> {
        age_at(
            self.frame.produced_qpc,
            self.frame.qpc_frequency,
            now(),
            frequency(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: u64 = 10_000_000; // 10 MHz, the usual QPC frequency on Windows 10 and later

    fn us(n: u64) -> Duration {
        Duration::from_micros(n)
    }

    fn stamps(produced: u64, sent: u64, received: u64) -> Stamps {
        Stamps {
            produced: Some(produced),
            sent: Some(sent),
            received: Some(received),
            plugin_frequency: Some(F),
        }
    }

    // ---- conversion ----

    #[test]
    fn ticks_convert_exactly_at_common_and_odd_frequencies() {
        assert_eq!(ticks_to_duration(0, F), Some(Duration::ZERO));
        assert_eq!(ticks_to_duration(1, F), Some(Duration::from_nanos(100)));
        assert_eq!(ticks_to_duration(F, F), Some(Duration::from_secs(1)));
        assert_eq!(
            ticks_to_duration(16_667, F),
            Some(us(1_666) + Duration::from_nanos(700))
        );
        // 3,579,545 Hz (ACPI PM timer, older systems): one tick is 279.365... ns.
        assert_eq!(
            ticks_to_duration(1, 3_579_545),
            Some(Duration::from_nanos(279))
        );
        assert_eq!(
            ticks_to_duration(3_579_545 * 5 + 1, 3_579_545),
            Some(Duration::new(5, 279))
        );
        assert_eq!(ticks_to_duration(5, 0), None, "zero frequency");
    }

    #[test]
    fn conversion_cannot_overflow() {
        assert_eq!(
            ticks_to_duration(u64::MAX, 1),
            Some(Duration::from_secs(u64::MAX))
        );
        let d = ticks_to_duration(u64::MAX, u64::MAX - 1).unwrap();
        assert_eq!(d.as_secs(), 1);
        assert!(ticks_to_duration(u64::MAX, F).is_some());
        assert!(ticks_to_duration(u64::MAX - 1, u64::MAX).unwrap() < Duration::from_secs(1));
    }

    // ---- valid stamps ----

    #[test]
    fn valid_stamps_give_processing_transport_and_age() {
        // FRAME_END at t, sent 4 ms later, received 0.3 ms after that.
        let t = 123_456_789_000;
        let timing = evaluate(stamps(t, t + 40_000, t + 43_000), Some(F)).unwrap();
        assert_eq!(timing.plugin_processing, us(4_000));
        assert_eq!(timing.transport, us(300));
        assert_eq!(timing.age_at_receive, us(4_300));
        assert_eq!(
            timing.age_at_receive,
            timing.plugin_processing + timing.transport
        );
    }

    #[test]
    fn equal_stamps_are_valid_zero_durations() {
        let timing = evaluate(stamps(7, 7, 7), Some(F)).unwrap();
        assert_eq!(timing.age_at_receive, Duration::ZERO);
    }

    #[test]
    fn consecutive_frames_are_evaluated_independently() {
        // 60 Hz frames, each with its own stamps; one was held back 30 ms by the plugin.
        let base = 5_000_000_000u64;
        let frames = [
            (base, base + 1_000, base + 3_000),
            (base + 166_670, base + 167_500, base + 170_000),
            (base + 333_340, base + 633_340, base + 636_000),
            (base + 500_010, base + 501_000, base + 502_000),
        ];
        let ages: Vec<Duration> = frames
            .iter()
            .map(|&(p, s, r)| evaluate(stamps(p, s, r), Some(F)).unwrap().age_at_receive)
            .collect();
        assert_eq!(ages, [us(300), us(333), us(30_266), us(199)]);
        let processing = evaluate(stamps(frames[2].0, frames[2].1, frames[2].2), Some(F))
            .unwrap()
            .plugin_processing;
        assert_eq!(processing, us(30_000));
    }

    #[test]
    fn age_at_a_later_time() {
        let produced = Some(1_000_000);
        assert_eq!(
            age_at(produced, Some(F), Some(1_000_000 + 1_500_000), Some(F)),
            Ok(us(150_000))
        );
    }

    // ---- invalid stamps never become a low latency ----

    #[test]
    fn backwards_stamps_are_errors_not_latencies() {
        assert_eq!(
            evaluate(stamps(100, 99, 200), Some(F)),
            Err(TimingError::NotMonotonic {
                earlier: Stamp::Produced,
                later: Stamp::Sent
            })
        );
        assert_eq!(
            evaluate(stamps(100, 200, 150), Some(F)),
            Err(TimingError::NotMonotonic {
                earlier: Stamp::Sent,
                later: Stamp::Received
            })
        );
        // A received stamp before the produced one is caught at the send step first.
        assert!(evaluate(stamps(500, 600, 400), Some(F)).is_err());
        assert_eq!(
            age_at(Some(500), Some(F), Some(499), Some(F)),
            Err(TimingError::NotMonotonic {
                earlier: Stamp::Produced,
                later: Stamp::Now
            })
        );
    }

    #[test]
    fn missing_stamps_are_reported_by_name() {
        let full = stamps(1, 2, 3);
        for (stamps, missing) in [
            (
                Stamps {
                    produced: None,
                    ..full
                },
                Stamp::Produced,
            ),
            (Stamps { sent: None, ..full }, Stamp::Sent),
            (
                Stamps {
                    received: None,
                    ..full
                },
                Stamp::Received,
            ),
            (
                Stamps {
                    plugin_frequency: None,
                    ..full
                },
                Stamp::PluginFrequency,
            ),
        ] {
            assert_eq!(
                evaluate(stamps, Some(F)),
                Err(TimingError::Missing(missing))
            );
        }
        assert_eq!(
            evaluate(full, None),
            Err(TimingError::Missing(Stamp::LocalFrequency))
        );
        assert_eq!(
            evaluate(Stamps::default(), Some(F)),
            Err(TimingError::Missing(Stamp::Produced))
        );
        assert_eq!(
            age_at(Some(1), Some(F), None, Some(F)),
            Err(TimingError::Missing(Stamp::Now))
        );
    }

    #[test]
    fn a_wrong_or_zero_frequency_is_rejected() {
        let mismatch = Stamps {
            plugin_frequency: Some(3_579_545),
            ..stamps(1, 2, 3)
        };
        assert_eq!(
            evaluate(mismatch, Some(F)),
            Err(TimingError::FrequencyMismatch {
                plugin: 3_579_545,
                local: F
            })
        );
        let zero = Stamps {
            plugin_frequency: Some(0),
            ..stamps(1, 2, 3)
        };
        assert_eq!(
            evaluate(zero, Some(F)),
            Err(TimingError::Missing(Stamp::PluginFrequency))
        );
        assert_eq!(
            evaluate(stamps(1, 2, 3), Some(0)),
            Err(TimingError::Missing(Stamp::LocalFrequency))
        );
    }

    #[test]
    fn implausibly_long_spans_are_rejected() {
        let over = 61 * F;
        assert!(matches!(
            evaluate(stamps(0, 1, over), Some(F)),
            Err(TimingError::Implausible { .. })
        ));
        assert!(matches!(
            age_at(Some(0), Some(F), Some(over), Some(F)),
            Err(TimingError::Implausible { .. })
        ));
        // Exactly the limit is still a measurement.
        assert_eq!(
            evaluate(stamps(0, 1, 60 * F), Some(F)).map(|t| t.age_at_receive),
            Ok(MAX_PLAUSIBLE_SPAN)
        );
    }

    // ---- this process's clock ----

    #[cfg(windows)]
    #[test]
    fn the_local_clock_is_available_and_monotonic() {
        let frequency = frequency().expect("QPC frequency");
        assert!(frequency >= 1_000_000, "at least 1 MHz: {frequency}");
        let mut previous = now().expect("QPC value");
        for _ in 0..1_000 {
            let next = now().unwrap();
            assert!(next >= previous);
            previous = next;
        }
        let before = now().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let slept = ticks_to_duration(now().unwrap() - before, frequency).unwrap();
        assert!(slept >= Duration::from_millis(19), "{slept:?}");
        assert!(slept < Duration::from_secs(2), "{slept:?}");
    }
}
