//! Transport-independent domain types for RoadPilot telemetry.
//!
//! This crate does not read the SCS SDK or any IPC transport. Those adapters
//! will be added after the protocol and transport decisions are approved.

use serde::{Deserialize, Serialize};
use std::time::Instant;

#[cfg(windows)]
pub mod named_pipe;

/// Protocol identifier used by JSON frames.
pub const PROTOCOL_ID: &str = "roadpilot.telemetry";

/// Current candidate frame revision.
pub const FRAME_SCHEMA_VERSION: u16 = 1;

/// Maximum accepted serialized frame size.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Maximum JSON integer that is represented exactly by JavaScript consumers.
pub const MAX_SAFE_SEQUENCE: u64 = 9_007_199_254_740_991;

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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryValues {
    /// SCS truck speed in meters per second. Negative values indicate reverse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mps: Option<f32>,
    /// SCS engine RPM. Missing means unavailable, not zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_rpm: Option<f32>,
    /// Selected engine gear: positive forward, zero neutral, negative reverse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<i32>,
}

impl TelemetryValues {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.speed_mps.is_some_and(|value| !value.is_finite()) {
            return Err(ValidationError::NonFiniteSpeed);
        }
        if self
            .engine_rpm
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(ValidationError::InvalidEngineRpm);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryFrameV1 {
    pub protocol: String,
    pub schema_version: u16,
    /// Monotonically increasing within one plugin session.
    pub sequence: u64,
    pub game: GameId,
    pub plugin_state: PluginState,
    pub values: TelemetryValues,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationError {
    WrongProtocol,
    UnsupportedSchemaVersion,
    SequenceExceedsSafeInteger,
    NonFiniteSpeed,
    InvalidEngineRpm,
}

impl TelemetryFrameV1 {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.protocol != PROTOCOL_ID {
            return Err(ValidationError::WrongProtocol);
        }
        if self.schema_version != FRAME_SCHEMA_VERSION {
            return Err(ValidationError::UnsupportedSchemaVersion);
        }
        if self.sequence > MAX_SAFE_SEQUENCE {
            return Err(ValidationError::SequenceExceedsSafeInteger);
        }
        self.values.validate()
    }
}

#[derive(Debug)]
pub enum DecodeError {
    FrameTooLarge,
    InvalidJson(serde_json::Error),
    InvalidFrame(ValidationError),
}

/// Decode and validate a JSON frame. Unknown fields are tolerated so additive
/// optional fields can be introduced without breaking older readers.
pub fn decode_frame(bytes: &[u8]) -> Result<TelemetryFrameV1, DecodeError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(DecodeError::FrameTooLarge);
    }

    let frame: TelemetryFrameV1 =
        serde_json::from_slice(bytes).map_err(DecodeError::InvalidJson)?;
    frame.validate().map_err(DecodeError::InvalidFrame)?;
    Ok(frame)
}

/// A validated frame paired with the desktop reader's monotonic receive time.
/// The receive time is local metadata and must not be serialized onto the wire.
#[derive(Clone, Debug, PartialEq)]
pub struct ReceivedTelemetryFrame {
    pub frame: TelemetryFrameV1,
    pub received_at: Instant,
}

impl ReceivedTelemetryFrame {
    pub fn new(frame: TelemetryFrameV1, received_at: Instant) -> Result<Self, ValidationError> {
        frame.validate()?;
        Ok(Self { frame, received_at })
    }
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
