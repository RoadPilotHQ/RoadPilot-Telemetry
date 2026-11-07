//! RoadPilot telemetry protocol: frame model, decoding and validation.
//!
//! The wire format is described in `docs/PROTOCOL-V2.md`. Frames are JSON objects carrying
//! `protocol` and `schemaVersion`. Schema 2 is current; schema 1 frames (sent by older
//! plugins) are still accepted and upgraded to the same [`TelemetryFrame`] model with all
//! fields that v1 cannot carry left absent.
//!
//! This crate does not talk to the SCS SDK. The Windows transport is in [`named_pipe`].

use serde::{Deserialize, Serialize};
use std::time::Instant;

#[cfg(windows)]
pub mod named_pipe;
pub mod ordering;
pub mod snapshot;
pub mod v1;
pub mod v2;

pub use v2::{
    Inputs, Job, JobState, MAX_WHEELS, Motion, Navigation, Placement, TelemetryFrame, TruckConfig,
    TruckState, Vec3, WheelConfig,
};

/// Protocol identifier carried by every frame.
pub const PROTOCOL_ID: &str = "roadpilot.telemetry";

/// Schema written by the current plugin.
pub const CURRENT_SCHEMA_VERSION: u16 = 2;

/// Schemas this reader accepts. Unknown versions fail closed.
pub const SUPPORTED_SCHEMA_VERSIONS: [u16; 2] = [1, 2];

/// Maximum accepted serialized frame size.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Largest integer represented exactly by JavaScript consumers (2^53 - 1). Session ids,
/// sequence numbers and timestamps must not exceed it.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GameId {
    Ets2,
    Ats,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginState {
    Running,
    Paused,
}

/// Errors that reject a whole frame. Invalid individual values do not reject the frame;
/// they are removed and reported in [`DecodedFrame::dropped_fields`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationError {
    WrongProtocol,
    UnsupportedSchemaVersion(u64),
    SessionExceedsSafeInteger,
    SequenceExceedsSafeInteger,
    TimestampExceedsSafeInteger,
    /// A schema 2 frame without its mandatory session id.
    MissingSession,
}

#[derive(Debug)]
pub enum DecodeError {
    FrameTooLarge,
    InvalidJson(serde_json::Error),
    InvalidFrame(ValidationError),
}

/// A validated frame plus the names of values that were removed as invalid.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedFrame {
    pub frame: TelemetryFrame,
    /// Wire paths such as `truck.engineRpm`, for diagnostics.
    pub dropped_fields: Vec<&'static str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    protocol: String,
    schema_version: u64,
}

/// Decode and validate a JSON frame of any supported schema.
///
/// Unknown fields are ignored so that additive optional fields can be introduced without
/// breaking older readers.
pub fn decode_frame(bytes: &[u8]) -> Result<DecodedFrame, DecodeError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(DecodeError::FrameTooLarge);
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(DecodeError::InvalidJson)?;
    if envelope.protocol != PROTOCOL_ID {
        return Err(DecodeError::InvalidFrame(ValidationError::WrongProtocol));
    }
    let mut frame = match envelope.schema_version {
        1 => serde_json::from_slice::<v1::TelemetryFrameV1>(bytes)
            .map_err(DecodeError::InvalidJson)?
            .upgrade(),
        2 => serde_json::from_slice::<TelemetryFrame>(bytes).map_err(DecodeError::InvalidJson)?,
        other => {
            return Err(DecodeError::InvalidFrame(
                ValidationError::UnsupportedSchemaVersion(other),
            ));
        }
    };
    frame
        .validate_envelope()
        .map_err(DecodeError::InvalidFrame)?;
    let dropped_fields = frame.sanitize();
    Ok(DecodedFrame {
        frame,
        dropped_fields,
    })
}

/// A validated frame paired with the desktop reader's monotonic receive time.
/// The receive time is local metadata and is never serialized onto the wire.
#[derive(Clone, Debug, PartialEq)]
pub struct ReceivedTelemetryFrame {
    pub frame: TelemetryFrame,
    pub dropped_fields: Vec<&'static str>,
    pub received_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    NotConfigured,
    PluginNotDetected,
    GameStopped,
    Connecting,
    Connected,
    Stale,
    Disconnected,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = include_str!("../../schemas/fixtures/frame-v2-full.json");
    const MINIMAL: &str = include_str!("../../schemas/fixtures/frame-v2-minimal.json");
    const V1: &str = include_str!("../../schemas/fixtures/frame-v1.json");

    #[test]
    fn decodes_the_shared_full_fixture() {
        let decoded = decode_frame(FULL.as_bytes()).unwrap();
        assert!(
            decoded.dropped_fields.is_empty(),
            "{:?}",
            decoded.dropped_fields
        );
        let f = decoded.frame;
        assert_eq!(f.schema_version, 2);
        assert_eq!(f.session, Some(4_242_424_242));
        assert_eq!(f.sequence, 1234);
        assert_eq!(f.game, GameId::Ets2);
        assert_eq!(f.game_version.as_deref(), Some("1.21"));
        assert_eq!(f.plugin_state, PluginState::Running);
        assert_eq!(f.simulation_time_us, Some(98_765_432));
        assert_eq!(f.game_time_min, Some(2_000));
        let truck = f.truck.unwrap();
        assert_eq!(truck.speed_mps, Some(22.5));
        assert_eq!(truck.gear, Some(11));
        assert_eq!(truck.engine_enabled, Some(true));
        assert_eq!(truck.cruise_control_mps, Some(22.222));
        assert_eq!(truck.retarder_level, Some(1));
        let placement = f.placement.unwrap();
        assert_eq!(placement.x, -31_234.567_8);
        assert_eq!(placement.heading, 0.25);
        let job = f.job.unwrap();
        assert_eq!(job.state, JobState::Active);
        assert_eq!(job.source_city.as_deref(), Some("Hamburg"));
        assert_eq!(job.destination_city.as_deref(), Some("Praha \"Hlavní\""));
        assert_eq!(job.income, Some(24_360));
        let config = f.truck_config.unwrap();
        assert_eq!(config.fuel_capacity_l, Some(800.0));
        assert_eq!(config.wheel_count, Some(4));
        let hook = config.hook_position_m.unwrap();
        assert_eq!((hook.x, hook.y, hook.z), (0.0, 1.0, 1.25));
        let wheels = config.wheels.unwrap();
        assert_eq!(wheels.len(), 4);
        let front_left = wheels[0];
        let pos = front_left.position_m.unwrap();
        assert_eq!((pos.x, pos.y, pos.z), (-0.875, 0.5, -1.75));
        assert_eq!(front_left.radius_m, Some(0.5));
        assert_eq!(front_left.steerable, Some(true));
        assert_eq!(front_left.powered, Some(false));
        assert_eq!(wheels[3].steerable, Some(false));
        assert_eq!(wheels[3].powered, Some(true));
        assert_eq!(f.navigation.unwrap().speed_limit_mps, Some(22.222));
        assert_eq!(f.motion.unwrap().wheel_steering_rot, Some(-0.03));
        assert_eq!(f.inputs.unwrap().steering_input, Some(-0.125));
    }

    #[test]
    fn decodes_the_minimal_fixture_with_everything_absent() {
        let f = decode_frame(MINIMAL.as_bytes()).unwrap().frame;
        assert_eq!(f.session, Some(1));
        assert!(f.truck.is_none() && f.placement.is_none() && f.job.is_none());
        assert_eq!(f.plugin_state, PluginState::Paused);
    }

    #[test]
    fn upgrades_v1_frames_without_inventing_fields() {
        let f = decode_frame(V1.as_bytes()).unwrap().frame;
        assert_eq!(f.schema_version, 1);
        assert_eq!(f.session, None);
        assert_eq!(f.game, GameId::Ats);
        let truck = f.truck.unwrap();
        assert_eq!(truck.speed_mps, Some(-2.5));
        assert_eq!(truck.engine_rpm, Some(800.0));
        assert_eq!(truck.gear, Some(-1));
        assert_eq!(truck.fuel_l, None);
        assert!(f.placement.is_none() && f.job.is_none() && f.truck_config.is_none());
    }

    #[test]
    fn rejects_unknown_versions_and_wrong_protocol() {
        let v3 = MINIMAL.replace("\"schemaVersion\": 2", "\"schemaVersion\": 3");
        assert!(matches!(
            decode_frame(v3.as_bytes()),
            Err(DecodeError::InvalidFrame(
                ValidationError::UnsupportedSchemaVersion(3)
            ))
        ));
        let other = MINIMAL.replace("roadpilot.telemetry", "something.else");
        assert!(matches!(
            decode_frame(other.as_bytes()),
            Err(DecodeError::InvalidFrame(ValidationError::WrongProtocol))
        ));
    }

    #[test]
    fn rejects_v2_frames_without_session_or_with_unsafe_integers() {
        let no_session = MINIMAL.replace("\"session\": 1,", "");
        assert!(matches!(
            decode_frame(no_session.as_bytes()),
            Err(DecodeError::InvalidFrame(ValidationError::MissingSession))
        ));
        let big = MINIMAL.replace("\"sequence\": 7", "\"sequence\": 9007199254740992");
        assert!(matches!(
            decode_frame(big.as_bytes()),
            Err(DecodeError::InvalidFrame(
                ValidationError::SequenceExceedsSafeInteger
            ))
        ));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let extended = MINIMAL.replace(
            "\"pluginState\"",
            "\"futureField\": {\"a\": 1}, \"pluginState\"",
        );
        assert!(decode_frame(extended.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_oversized_and_malformed_frames() {
        let big = vec![b' '; MAX_FRAME_BYTES + 1];
        assert!(matches!(
            decode_frame(&big),
            Err(DecodeError::FrameTooLarge)
        ));
        assert!(matches!(
            decode_frame(b"{not json"),
            Err(DecodeError::InvalidJson(_))
        ));
        // A wrong type for a known field is a malformed frame, not a dropped value.
        let wrong_type = MINIMAL.replace("\"sequence\": 7", "\"sequence\": \"7\"");
        assert!(matches!(
            decode_frame(wrong_type.as_bytes()),
            Err(DecodeError::InvalidJson(_))
        ));
    }
}
