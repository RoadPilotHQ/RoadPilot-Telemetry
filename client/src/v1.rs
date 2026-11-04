//! Schema 1 frames, sent by the first plugin release. Accepted for compatibility and
//! upgraded to the current [`TelemetryFrame`] model.

use crate::{GameId, PluginState, TelemetryFrame, TruckState};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryValuesV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mps: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_rpm: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryFrameV1 {
    pub protocol: String,
    pub schema_version: u16,
    pub sequence: u64,
    pub game: GameId,
    pub plugin_state: PluginState,
    pub values: TelemetryValuesV1,
}

impl TelemetryFrameV1 {
    /// Maps the three v1 values; every field v1 cannot carry stays absent.
    pub fn upgrade(self) -> TelemetryFrame {
        let mut frame = TelemetryFrame::new(self.sequence, self.game, self.plugin_state);
        frame.protocol = self.protocol;
        frame.schema_version = SCHEMA_VERSION;
        frame.truck = Some(TruckState {
            speed_mps: self.values.speed_mps,
            engine_rpm: self.values.engine_rpm,
            gear: self.values.gear,
            ..TruckState::default()
        });
        frame
    }
}
