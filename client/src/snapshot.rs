//! One-line, copyable snapshot of the newest received frame, for manual
//! in-game measurements (see `docs/FRAME-SNAPSHOT.md`).
//!
//! The snapshot copies values from the decoded frame unchanged. Numbers are
//! written by `serde_json` in their shortest round-trip form with a decimal
//! point, independent of the system locale: `f64` positions keep every digit
//! the plugin sent, `f32` values are written as `f32` (no widening digits).
//! Nothing is rounded, converted or estimated.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::ReceivedTelemetryFrame;
use crate::v2::{Placement, Vec3};

/// Version of the snapshot line layout.
pub const SNAPSHOT_VERSION: u32 = 1;

/// Identity of a frame: plugin session and sequence number.
pub type FrameId = (Option<u64>, u64);

/// How the snapshot relates to the frames the reader has.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    /// No frame has been received yet.
    NoFrame,
    /// A new frame, received recently.
    Fresh,
    /// The same frame as the previous snapshot: no new frame arrived since.
    Repeated,
    /// The newest frame is older than the stale limit (game paused, closed,
    /// or the plugin stopped sending).
    Stale,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotFrame<'a> {
    protocol: &'a str,
    schema_version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<u64>,
    sequence: u64,
    game: crate::GameId,
    #[serde(skip_serializing_if = "Option::is_none")]
    game_version: Option<&'a str>,
    plugin_state: crate::PluginState,
    #[serde(skip_serializing_if = "Option::is_none")]
    simulation_time_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placement: Option<Placement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed_mps: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_velocity_mps: Option<Vec3>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotLine<'a> {
    kind: &'static str,
    snapshot_version: u32,
    status: SnapshotStatus,
    /// Milliseconds since the frame was received by this reader.
    #[serde(skip_serializing_if = "Option::is_none")]
    age_ms: Option<u64>,
    /// Fields the measurement needs that this frame does not contain.
    missing: Vec<&'static str>,
    /// Fields the decoder removed from this frame as invalid.
    dropped_fields: &'a [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<SnapshotFrame<'a>>,
}

/// The snapshot line and the identity of the frame it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub line: String,
    pub status: SnapshotStatus,
    pub frame: Option<FrameId>,
}

/// Builds the snapshot line for the newest frame.
///
/// `previous` is the frame shown by the previous snapshot; if the newest
/// frame is the same, the status is [`SnapshotStatus::Repeated`]. A frame
/// older than `stale_after` is [`SnapshotStatus::Stale`] (stale wins over
/// repeated).
#[must_use]
pub fn snapshot(
    latest: Option<&ReceivedTelemetryFrame>,
    now: Instant,
    stale_after: Duration,
    previous: Option<FrameId>,
) -> Snapshot {
    let Some(received) = latest else {
        let line = SnapshotLine {
            kind: "roadpilot.telemetry.snapshot",
            snapshot_version: SNAPSHOT_VERSION,
            status: SnapshotStatus::NoFrame,
            age_ms: None,
            missing: Vec::new(),
            dropped_fields: &[],
            frame: None,
        };
        return Snapshot {
            line: to_line(&line),
            status: SnapshotStatus::NoFrame,
            frame: None,
        };
    };
    let f = &received.frame;
    let id: FrameId = (f.session, f.sequence);
    let age = now.saturating_duration_since(received.received_at);
    let status = if age > stale_after {
        SnapshotStatus::Stale
    } else if previous == Some(id) {
        SnapshotStatus::Repeated
    } else {
        SnapshotStatus::Fresh
    };
    let speed_mps = f.truck.and_then(|t| t.speed_mps);
    let mut missing = Vec::new();
    if f.session.is_none() {
        missing.push("session");
    }
    if f.game_version.is_none() {
        missing.push("gameVersion");
    }
    if f.placement.is_none() {
        missing.push("placement");
    }
    if speed_mps.is_none() {
        missing.push("truck.speedMps");
    }
    let line = SnapshotLine {
        kind: "roadpilot.telemetry.snapshot",
        snapshot_version: SNAPSHOT_VERSION,
        status,
        age_ms: Some(u64::try_from(age.as_millis()).unwrap_or(u64::MAX)),
        missing,
        dropped_fields: &received.dropped_fields,
        frame: Some(SnapshotFrame {
            protocol: &f.protocol,
            schema_version: f.schema_version,
            session: f.session,
            sequence: f.sequence,
            game: f.game,
            game_version: f.game_version.as_deref(),
            plugin_state: f.plugin_state,
            simulation_time_us: f.simulation_time_us,
            local_scale: f.local_scale,
            placement: f.placement,
            speed_mps,
            local_velocity_mps: f.motion.as_ref().and_then(|m| m.local_velocity_mps),
        }),
    };
    Snapshot {
        line: to_line(&line),
        status,
        frame: Some(id),
    }
}

fn to_line(line: &SnapshotLine<'_>) -> String {
    // Serializing plain structs of numbers, strings and enums cannot fail;
    // fall back to a valid line anyway instead of panicking.
    serde_json::to_string(line).unwrap_or_else(|_| {
        "{\"kind\":\"roadpilot.telemetry.snapshot\",\"status\":\"error\"}".to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode_frame;

    /// A synthetic schema 2 frame (invented values, not a recording).
    fn frame(extra: &str) -> ReceivedTelemetryFrame {
        let json = format!(
            "{{\"protocol\":\"roadpilot.telemetry\",\"schemaVersion\":2,\"session\":42,\"sequence\":7,\"game\":\"ats\",\"gameVersion\":\"1.5\",\"pluginState\":\"running\",\"simulationTimeUs\":1000,\"localScale\":19.5{extra}}}"
        );
        let decoded = decode_frame(json.as_bytes()).expect("valid synthetic frame");
        ReceivedTelemetryFrame {
            frame: decoded.frame,
            dropped_fields: decoded.dropped_fields,
            received_at: Instant::now(),
        }
    }

    const FULL: &str = ",\"truck\":{\"speedMps\":0.0125},\"motion\":{\"localVelocityMps\":{\"x\":0.001,\"y\":0,\"z\":-0.0125}},\"placement\":{\"x\":-12345.678901234567,\"y\":12.25,\"z\":98765.4321,\"heading\":0.1,\"pitch\":-0.0021,\"roll\":0.00037}";

    fn parse(line: &str) -> serde_json::Value {
        serde_json::from_str(line).expect("the snapshot is valid JSON")
    }

    #[test]
    fn copies_every_needed_field_without_rounding() {
        let f = frame(FULL);
        let s = snapshot(Some(&f), f.received_at, Duration::from_secs(2), None);
        assert_eq!(s.status, SnapshotStatus::Fresh);
        assert_eq!(s.frame, Some((Some(42), 7)));
        assert!(!s.line.contains('\n'), "one line");
        let v = parse(&s.line);
        let fr = &v["frame"];
        assert_eq!(fr["session"], 42);
        assert_eq!(fr["sequence"], 7);
        assert_eq!(fr["game"], "ats");
        assert_eq!(fr["gameVersion"], "1.5");
        // Doubles keep all digits; the text equals the input text.
        assert!(s.line.contains("\"x\":-12345.678901234567"), "{}", s.line);
        assert!(s.line.contains("\"z\":98765.4321"));
        assert!(s.line.contains("\"y\":12.25"));
        // Floats are written as floats: no invented widening digits.
        assert!(s.line.contains("\"heading\":0.1,"), "{}", s.line);
        assert!(s.line.contains("\"pitch\":-0.0021"));
        assert!(s.line.contains("\"roll\":0.00037"));
        assert!(s.line.contains("\"speedMps\":0.0125"));
        assert!(s.line.contains("\"localScale\":19.5"));
        assert_eq!(v["missing"], serde_json::json!([]));
        // Round trip: parsing the snapshot gives back the same f64 bits.
        assert_eq!(fr["placement"]["x"].as_f64(), Some(-12345.678901234567));
    }

    #[test]
    fn numbers_use_a_decimal_point_and_no_grouping() {
        let s = snapshot(
            Some(&frame(FULL)),
            Instant::now(),
            Duration::from_secs(2),
            None,
        );
        let v = parse(&s.line);
        // Every number in the placement is a JSON number (not a string), so a
        // decimal comma or a thousands separator would not parse.
        for k in ["x", "y", "z", "heading", "pitch", "roll"] {
            assert!(v["frame"]["placement"][k].is_number(), "{k}");
        }
        assert!(!s.line.contains("12345,"));
        assert!(!s.line.contains("98.765"));
    }

    #[test]
    fn reports_missing_fields_instead_of_inventing_them() {
        let f = frame("");
        let s = snapshot(Some(&f), f.received_at, Duration::from_secs(2), None);
        let v = parse(&s.line);
        assert_eq!(
            v["missing"],
            serde_json::json!(["placement", "truck.speedMps"])
        );
        assert!(v["frame"].get("placement").is_none());
        assert!(v["frame"].get("speedMps").is_none());
    }

    #[test]
    fn reports_dropped_invalid_placements() {
        // An implausible heading removes the whole placement in the decoder.
        let f =
            frame(",\"placement\":{\"x\":1,\"y\":2,\"z\":3,\"heading\":7,\"pitch\":0,\"roll\":0}");
        let s = snapshot(Some(&f), f.received_at, Duration::from_secs(2), None);
        let v = parse(&s.line);
        assert!(
            v["missing"]
                .as_array()
                .is_some_and(|m| m.contains(&serde_json::json!("placement")))
        );
        assert!(v["droppedFields"].as_array().is_some_and(|d| !d.is_empty()));
    }

    #[test]
    fn marks_repeated_and_stale_frames() {
        let f = frame(FULL);
        let first = snapshot(Some(&f), f.received_at, Duration::from_secs(2), None);
        let again = snapshot(Some(&f), f.received_at, Duration::from_secs(2), first.frame);
        assert_eq!(again.status, SnapshotStatus::Repeated);
        assert!(again.line.contains("\"status\":\"repeated\""));
        let late = f.received_at + Duration::from_secs(5);
        let stale = snapshot(Some(&f), late, Duration::from_secs(2), first.frame);
        assert_eq!(stale.status, SnapshotStatus::Stale);
        assert!(stale.line.contains("\"ageMs\":5000"));
    }

    #[test]
    fn without_a_frame_says_so() {
        let s = snapshot(None, Instant::now(), Duration::from_secs(2), None);
        assert_eq!(s.status, SnapshotStatus::NoFrame);
        assert_eq!(s.frame, None);
        let v = parse(&s.line);
        assert_eq!(v["status"], "no_frame");
        assert!(v.get("frame").is_none());
    }
}
