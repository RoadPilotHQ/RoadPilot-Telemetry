//! Schema 2 frame model. Field names carry their unit. Every value is optional: absent means
//! the game did not provide it (or it was removed as invalid), never zero.
//!
//! Values are raw SCS SDK quantities in SDK units and axes; conversions are left to
//! consumers. See `docs/PROTOCOL-V2.md` for units, ranges and coordinate conventions.

use crate::{GameId, MAX_SAFE_INTEGER, PROTOCOL_ID, PluginState, ValidationError};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u16 = 2;

/// Longest accepted string value in bytes (city, company, cargo and truck names).
pub const MAX_STRING_BYTES: usize = 256;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryFrame {
    pub protocol: String,
    pub schema_version: u16,
    /// Random id of one plugin initialization; changes when the game (re)loads the plugin.
    /// Mandatory in schema 2, absent for upgraded schema 1 frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<u64>,
    /// Strictly increasing within a session, across pipe reconnects.
    pub sequence: u64,
    pub game: GameId,
    /// SDK telemetry game version as `major.minor` (not the game's patch number).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    pub plugin_state: PluginState,
    /// QueryPerformanceCounter value when the game finished this frame (`FRAME_END`). QPC
    /// ticks; compare only with QPC values of the same machine. See [`crate::qpc`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produced_qpc: Option<u64>,
    /// QueryPerformanceCounter value immediately before the plugin serialized and wrote
    /// this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_qpc: Option<u64>,
    /// The plugin's QPC frequency in ticks per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qpc_frequency: Option<u64>,
    /// SDK simulation time of the frame in microseconds. May restart from zero when the
    /// game restarts its timers; ordering must use `sequence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation_time_us: Option<u64>,
    /// In-game minutes since 00:00 of the first in-game day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_time_min: Option<u32>,
    /// Map scale applied to distance and time (game seconds per real second).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truck: Option<TruckState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Inputs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truck_config: Option<TruckConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<Job>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation: Option<Navigation>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TruckState {
    /// Speedometer speed; negative while reversing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mps: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_rpm: Option<f32>,
    /// Engine gear: >0 forward, 0 neutral, <0 reverse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<i32>,
    /// Gear shown on the dashboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayed_gear: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub electric_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_l: Option<f32>,
    /// Game-estimated range with the current fuel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_range_km: Option<f32>,
    /// Game-provided average consumption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_consumption_l_per_km: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_warning: Option<bool>,
    /// Not supported by ATS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adblue_l: Option<f32>,
    /// Loosely approximated by the game, not simulated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oil_temperature_c: Option<f32>,
    /// Loosely approximated by the game, not simulated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_temperature_c: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oil_pressure_psi: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_voltage_v: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brake_air_pressure_psi: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brake_temperature_c: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parking_brake: Option<bool>,
    /// Engine brake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motor_brake: Option<bool>,
    /// 0 = off, up to `truckConfig.retarderSteps`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retarder_level: Option<u32>,
    /// Cruise control set speed; 0 means cruise control is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cruise_control_mps: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub odometer_km: Option<f32>,
}

/// Driver inputs. `*Input` is what the player gives, `*Effective` what the simulation uses
/// (interpolation, cruise control, automatic clutch). Steering is <-1;1> and counterclockwise
/// (positive = left); throttle, brake and clutch are <0;1>.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inputs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering_input: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throttle_input: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brake_input: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clutch_input: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering_effective: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throttle_effective: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brake_effective: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clutch_effective: Option<f32>,
}

/// Vector in SDK axes. Vehicle space: X right, Y up, Z backwards.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Motion {
    /// Vehicle-space linear velocity (X right, Y up, Z backwards).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_velocity_mps: Option<Vec3>,
    /// Vehicle-space linear acceleration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_acceleration_mps2: Option<Vec3>,
    /// Vehicle-space angular velocity in rotations per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_angular_velocity_rps: Option<Vec3>,
    /// Road-wheel steering of the first steerable wheel in rotations, <-0.25;0.25>,
    /// counterclockwise from above (positive = left). A physical angle, not driver input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel_steering_rot: Option<f32>,
}

/// World placement of the truck. World space: X east, Y up, Z south, in meters.
/// Orientation in SDK unit ranges: heading <0;1) counterclockwise from north
/// (0.25 = west), pitch <-0.25;0.25> (positive up), roll <-0.5;0.5>.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub heading: f32,
    pub pitch: f32,
    pub roll: f32,
}

/// Static truck configuration (changes only when the truck changes).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TruckConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_capacity_l: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adblue_capacity_l: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpm_limit: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_gears: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_gears: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retarder_steps: Option<u32>,
    /// Wheel count reported by the game; may exceed `wheels.len()` (see `MAX_WHEELS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel_count: Option<u32>,
    /// Trailer hook position in raw SDK vehicle space (X right, Y up, Z backwards), metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_position_m: Option<Vec3>,
    /// Wheel geometry by SDK wheel index. Absent when the game sent no wheel count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheels: Option<Vec<WheelConfig>>,
}

/// Most wheels per truck carried by the protocol.
pub const MAX_WHEELS: usize = 14;

/// Static geometry of one wheel, raw SDK vehicle space.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelConfig {
    /// Wheel centre in vehicle space (X right, Y up, Z backwards), metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_m: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steerable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub powered: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    /// The game reported that no job is active.
    None,
    Active,
    /// The last job was delivered; stays until the next job starts.
    Delivered,
    /// The last job was cancelled; stays until the next job starts.
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_mass_kg: Option<f32>,
    /// Total cargo damage <0;1>.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_damage: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_loaded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_company: Option<String>,
    /// Reward in the game currency (EUR in ETS2, USD in ATS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub income: Option<u64>,
    /// Absolute in-game minute at which the delivery window ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_time_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_distance_km: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_market: Option<String>,
    /// Revenue from the `job.delivered` gameplay event, in game currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_revenue: Option<i64>,
    /// Penalty from the `job.cancelled` gameplay event, in game currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_penalty: Option<i64>,
}

/// Route Advisor values as provided by the game.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Navigation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f32>,
    /// Advisor ETA in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_s: Option<f32>,
    /// Respects the in-game "Route Advisor speed limit" option. The meaning of 0 is not
    /// documented by the SDK and is passed through unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_limit_mps: Option<f32>,
}

impl TelemetryFrame {
    pub fn new(sequence: u64, game: GameId, plugin_state: PluginState) -> Self {
        Self {
            protocol: PROTOCOL_ID.to_owned(),
            schema_version: SCHEMA_VERSION,
            session: None,
            sequence,
            game,
            game_version: None,
            plugin_state,
            produced_qpc: None,
            sent_qpc: None,
            qpc_frequency: None,
            simulation_time_us: None,
            game_time_min: None,
            local_scale: None,
            truck: None,
            inputs: None,
            motion: None,
            placement: None,
            truck_config: None,
            job: None,
            navigation: None,
        }
    }

    /// Checks that reject the whole frame.
    pub fn validate_envelope(&self) -> Result<(), ValidationError> {
        if self.protocol != PROTOCOL_ID {
            return Err(ValidationError::WrongProtocol);
        }
        if !crate::SUPPORTED_SCHEMA_VERSIONS.contains(&self.schema_version) {
            return Err(ValidationError::UnsupportedSchemaVersion(
                self.schema_version.into(),
            ));
        }
        match self.session {
            None if self.schema_version >= 2 => return Err(ValidationError::MissingSession),
            Some(s) if s > MAX_SAFE_INTEGER => {
                return Err(ValidationError::SessionExceedsSafeInteger);
            }
            _ => {}
        }
        if self.sequence > MAX_SAFE_INTEGER {
            return Err(ValidationError::SequenceExceedsSafeInteger);
        }
        if self
            .simulation_time_us
            .is_some_and(|t| t > MAX_SAFE_INTEGER)
        {
            return Err(ValidationError::TimestampExceedsSafeInteger);
        }
        Ok(())
    }

    /// Removes non-finite and implausible values and returns their wire paths. The bounds
    /// are corruption guards, wider than anything the game produces; they are not game limits.
    pub fn sanitize(&mut self) -> Vec<&'static str> {
        let mut d = Dropped::default();
        if self.game_version.as_ref().is_some_and(|v| v.len() > 32) {
            self.game_version = None;
            d.0.push("gameVersion");
        }
        // A QPC value of 0 or beyond the exactly representable range, and a zero frequency,
        // cannot be real; they are removed like any other corrupt value.
        d.qpc(&mut self.produced_qpc, 1, "producedQpc");
        d.qpc(&mut self.sent_qpc, 1, "sentQpc");
        d.qpc(&mut self.qpc_frequency, 1, "qpcFrequency");
        d.f32(&mut self.local_scale, 0.0, 1_000.0, "localScale");
        if let Some(t) = &mut self.truck {
            d.f32(&mut t.speed_mps, -150.0, 150.0, "truck.speedMps");
            d.f32(&mut t.engine_rpm, 0.0, 20_000.0, "truck.engineRpm");
            d.i32(&mut t.gear, -32, 64, "truck.gear");
            d.i32(&mut t.displayed_gear, -32, 64, "truck.displayedGear");
            d.f32(&mut t.fuel_l, 0.0, 10_000.0, "truck.fuelL");
            d.f32(&mut t.fuel_range_km, 0.0, 100_000.0, "truck.fuelRangeKm");
            d.f32(
                &mut t.fuel_consumption_l_per_km,
                0.0,
                10.0,
                "truck.fuelConsumptionLPerKm",
            );
            d.f32(&mut t.adblue_l, 0.0, 2_000.0, "truck.adblueL");
            d.f32(
                &mut t.oil_temperature_c,
                -100.0,
                500.0,
                "truck.oilTemperatureC",
            );
            d.f32(
                &mut t.water_temperature_c,
                -100.0,
                500.0,
                "truck.waterTemperatureC",
            );
            d.f32(
                &mut t.oil_pressure_psi,
                0.0,
                1_000.0,
                "truck.oilPressurePsi",
            );
            d.f32(
                &mut t.battery_voltage_v,
                0.0,
                100.0,
                "truck.batteryVoltageV",
            );
            d.f32(
                &mut t.brake_air_pressure_psi,
                0.0,
                1_000.0,
                "truck.brakeAirPressurePsi",
            );
            d.f32(
                &mut t.brake_temperature_c,
                -100.0,
                2_000.0,
                "truck.brakeTemperatureC",
            );
            d.u32(&mut t.retarder_level, 64, "truck.retarderLevel");
            d.f32(
                &mut t.cruise_control_mps,
                0.0,
                150.0,
                "truck.cruiseControlMps",
            );
            d.f32(&mut t.odometer_km, 0.0, 100_000_000.0, "truck.odometerKm");
        }
        if let Some(i) = &mut self.inputs {
            d.unit(&mut i.steering_input, -1.0, "inputs.steeringInput");
            d.unit(&mut i.throttle_input, 0.0, "inputs.throttleInput");
            d.unit(&mut i.brake_input, 0.0, "inputs.brakeInput");
            d.unit(&mut i.clutch_input, 0.0, "inputs.clutchInput");
            d.unit(&mut i.steering_effective, -1.0, "inputs.steeringEffective");
            d.unit(&mut i.throttle_effective, 0.0, "inputs.throttleEffective");
            d.unit(&mut i.brake_effective, 0.0, "inputs.brakeEffective");
            d.unit(&mut i.clutch_effective, 0.0, "inputs.clutchEffective");
        }
        if let Some(m) = &mut self.motion {
            d.vec3(&mut m.local_velocity_mps, 200.0, "motion.localVelocityMps");
            d.vec3(
                &mut m.local_acceleration_mps2,
                1_000.0,
                "motion.localAccelerationMps2",
            );
            d.vec3(
                &mut m.local_angular_velocity_rps,
                100.0,
                "motion.localAngularVelocityRps",
            );
            d.f32(
                &mut m.wheel_steering_rot,
                -0.25 - TOLERANCE,
                0.25 + TOLERANCE,
                "motion.wheelSteeringRot",
            );
        }
        if self.placement.is_some_and(|p| !placement_valid(&p)) {
            self.placement = None;
            d.0.push("placement");
        }
        if let Some(c) = &mut self.truck_config {
            d.string(&mut c.brand, "truckConfig.brand");
            d.string(&mut c.name, "truckConfig.name");
            d.f32(
                &mut c.fuel_capacity_l,
                0.0,
                10_000.0,
                "truckConfig.fuelCapacityL",
            );
            d.f32(
                &mut c.adblue_capacity_l,
                0.0,
                2_000.0,
                "truckConfig.adblueCapacityL",
            );
            d.f32(&mut c.rpm_limit, 0.0, 20_000.0, "truckConfig.rpmLimit");
            d.u32(&mut c.forward_gears, 64, "truckConfig.forwardGears");
            d.u32(&mut c.reverse_gears, 32, "truckConfig.reverseGears");
            d.u32(&mut c.retarder_steps, 64, "truckConfig.retarderSteps");
            d.u32(&mut c.wheel_count, 4_096, "truckConfig.wheelCount");
            d.vec3(&mut c.hook_position_m, 50.0, "truckConfig.hookPositionM");
            if c.wheels.as_ref().is_some_and(|w| w.len() > MAX_WHEELS) {
                c.wheels = None;
                d.0.push("truckConfig.wheels");
            }
            for wheel in c.wheels.iter_mut().flatten() {
                d.vec3(&mut wheel.position_m, 50.0, "truckConfig.wheels.positionM");
                d.f32(&mut wheel.radius_m, 0.05, 3.0, "truckConfig.wheels.radiusM");
            }
        }
        if let Some(j) = &mut self.job {
            d.string(&mut j.cargo, "job.cargo");
            d.f32(&mut j.cargo_mass_kg, 0.0, 1_000_000.0, "job.cargoMassKg");
            d.f32(&mut j.cargo_damage, 0.0, 1.0, "job.cargoDamage");
            d.string(&mut j.source_city, "job.sourceCity");
            d.string(&mut j.source_company, "job.sourceCompany");
            d.string(&mut j.destination_city, "job.destinationCity");
            d.string(&mut j.destination_company, "job.destinationCompany");
            d.string(&mut j.job_market, "job.jobMarket");
            if j.income.is_some_and(|v| v > MAX_SAFE_INTEGER) {
                j.income = None;
                d.0.push("job.income");
            }
            d.u32(&mut j.planned_distance_km, 100_000, "job.plannedDistanceKm");
            d.i64(&mut j.delivered_revenue, "job.deliveredRevenue");
            d.i64(&mut j.cancel_penalty, "job.cancelPenalty");
        }
        if let Some(n) = &mut self.navigation {
            d.f32(
                &mut n.distance_m,
                0.0,
                100_000_000.0,
                "navigation.distanceM",
            );
            d.f32(&mut n.time_s, 0.0, 100_000_000.0, "navigation.timeS");
            d.f32(
                &mut n.speed_limit_mps,
                0.0,
                150.0,
                "navigation.speedLimitMps",
            );
        }
        d.0
    }
}

/// Inputs may overshoot their documented range by float noise; within this tolerance they
/// are clamped, beyond it they are dropped.
const TOLERANCE: f32 = 1e-3;

fn placement_valid(p: &Placement) -> bool {
    let finite = p.x.is_finite()
        && p.y.is_finite()
        && p.z.is_finite()
        && p.heading.is_finite()
        && p.pitch.is_finite()
        && p.roll.is_finite();
    finite
        && p.x.abs() <= 10_000_000.0
        && p.z.abs() <= 10_000_000.0
        && p.y.abs() <= 100_000.0
        && (0.0..=1.0).contains(&p.heading)
        && (-0.25 - TOLERANCE..=0.25 + TOLERANCE).contains(&p.pitch)
        && (-0.5 - TOLERANCE..=0.5 + TOLERANCE).contains(&p.roll)
}

#[derive(Default)]
struct Dropped(Vec<&'static str>);

impl Dropped {
    fn f32(&mut self, v: &mut Option<f32>, min: f32, max: f32, name: &'static str) {
        if v.is_some_and(|x| !(x.is_finite() && (min..=max).contains(&x))) {
            *v = None;
            self.0.push(name);
        }
    }

    /// Input in <min;1>, clamping float noise.
    fn unit(&mut self, v: &mut Option<f32>, min: f32, name: &'static str) {
        self.f32(v, min - TOLERANCE, 1.0 + TOLERANCE, name);
        if let Some(x) = v {
            *x = x.clamp(min, 1.0);
        }
    }

    fn i32(&mut self, v: &mut Option<i32>, min: i32, max: i32, name: &'static str) {
        if v.is_some_and(|x| !(min..=max).contains(&x)) {
            *v = None;
            self.0.push(name);
        }
    }

    /// A QPC tick count or frequency: at least `min` and exactly representable in JSON.
    fn qpc(&mut self, v: &mut Option<u64>, min: u64, name: &'static str) {
        if v.is_some_and(|x| !(min..=crate::MAX_SAFE_INTEGER).contains(&x)) {
            *v = None;
            self.0.push(name);
        }
    }

    fn u32(&mut self, v: &mut Option<u32>, max: u32, name: &'static str) {
        if v.is_some_and(|x| x > max) {
            *v = None;
            self.0.push(name);
        }
    }

    fn i64(&mut self, v: &mut Option<i64>, name: &'static str) {
        if v.is_some_and(|x| x.unsigned_abs() > MAX_SAFE_INTEGER) {
            *v = None;
            self.0.push(name);
        }
    }

    fn vec3(&mut self, v: &mut Option<Vec3>, limit: f32, name: &'static str) {
        let ok = |c: f32| c.is_finite() && c.abs() <= limit;
        if v.is_some_and(|p| !(ok(p.x) && ok(p.y) && ok(p.z))) {
            *v = None;
            self.0.push(name);
        }
    }

    fn string(&mut self, v: &mut Option<String>, name: &'static str) {
        if v.as_ref().is_some_and(|s| s.len() > MAX_STRING_BYTES) {
            *v = None;
            self.0.push(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> TelemetryFrame {
        let mut f = TelemetryFrame::new(1, GameId::Ets2, PluginState::Running);
        f.session = Some(9);
        f
    }

    #[test]
    fn drops_invalid_values_and_keeps_valid_ones() {
        let mut f = frame();
        f.truck = Some(TruckState {
            speed_mps: Some(f32::NAN),
            engine_rpm: Some(-1.0),
            gear: Some(12),
            fuel_l: Some(400.0),
            cruise_control_mps: Some(500.0),
            ..TruckState::default()
        });
        let dropped = f.sanitize();
        assert_eq!(
            dropped,
            vec![
                "truck.speedMps",
                "truck.engineRpm",
                "truck.cruiseControlMps"
            ]
        );
        let t = f.truck.unwrap();
        assert_eq!(
            (t.speed_mps, t.engine_rpm, t.cruise_control_mps),
            (None, None, None)
        );
        assert_eq!((t.gear, t.fuel_l), (Some(12), Some(400.0)));
    }

    #[test]
    fn clamps_input_noise_and_drops_out_of_range_inputs() {
        let mut f = frame();
        f.inputs = Some(Inputs {
            steering_input: Some(-1.0005),
            throttle_input: Some(1.5),
            brake_input: Some(0.25),
            ..Inputs::default()
        });
        assert_eq!(f.sanitize(), vec!["inputs.throttleInput"]);
        let i = f.inputs.unwrap();
        assert_eq!(i.steering_input, Some(-1.0));
        assert_eq!(i.brake_input, Some(0.25));
    }

    #[test]
    fn drops_the_whole_placement_when_any_component_is_invalid() {
        let mut f = frame();
        f.placement = Some(Placement {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            heading: 1.5,
            pitch: 0.0,
            roll: 0.0,
        });
        assert_eq!(f.sanitize(), vec!["placement"]);
        assert!(f.placement.is_none());
        f.placement = Some(Placement {
            x: f64::INFINITY,
            ..Placement::default()
        });
        assert_eq!(f.sanitize(), vec!["placement"]);
    }

    #[test]
    fn drops_invalid_vectors_strings_and_job_values() {
        let mut f = frame();
        f.motion = Some(Motion {
            local_velocity_mps: Some(Vec3 {
                x: 0.0,
                y: f32::NAN,
                z: -20.0,
            }),
            wheel_steering_rot: Some(0.1),
            ..Motion::default()
        });
        f.job = Some(Job {
            state: JobState::Active,
            cargo: Some("x".repeat(MAX_STRING_BYTES + 1)),
            cargo_mass_kg: Some(12_000.0),
            cargo_damage: Some(1.2),
            cargo_loaded: None,
            source_city: Some("Hamburg".into()),
            source_company: None,
            destination_city: None,
            destination_company: None,
            income: Some(u64::MAX),
            delivery_time_min: Some(3_000),
            planned_distance_km: None,
            job_market: None,
            delivered_revenue: None,
            cancel_penalty: None,
        });
        let dropped = f.sanitize();
        assert_eq!(
            dropped,
            vec![
                "motion.localVelocityMps",
                "job.cargo",
                "job.cargoDamage",
                "job.income"
            ]
        );
        let j = f.job.unwrap();
        assert_eq!(j.source_city.as_deref(), Some("Hamburg"));
        assert_eq!(j.cargo_mass_kg, Some(12_000.0));
        assert_eq!(f.motion.unwrap().wheel_steering_rot, Some(0.1));
    }

    #[test]
    fn zero_values_are_real_values() {
        let mut f = frame();
        f.truck = Some(TruckState {
            speed_mps: Some(0.0),
            gear: Some(0),
            cruise_control_mps: Some(0.0),
            ..TruckState::default()
        });
        f.navigation = Some(Navigation {
            speed_limit_mps: Some(0.0),
            ..Navigation::default()
        });
        assert!(f.sanitize().is_empty());
    }

    #[test]
    fn drops_invalid_wheel_values_individually_and_oversized_wheel_lists() {
        let mut f = frame();
        let wheel = |x: f32, r: f32| WheelConfig {
            position_m: Some(Vec3 { x, y: 0.5, z: 1.0 }),
            radius_m: Some(r),
            steerable: Some(false),
            powered: Some(true),
        };
        f.truck_config = Some(TruckConfig {
            wheel_count: Some(2),
            hook_position_m: Some(Vec3 {
                x: f32::NAN,
                y: 0.0,
                z: 0.0,
            }),
            wheels: Some(vec![wheel(f32::INFINITY, 0.5), wheel(0.9, 0.0)]),
            ..TruckConfig::default()
        });
        let dropped = f.sanitize();
        let c = f.truck_config.as_ref().unwrap();
        assert!(c.hook_position_m.is_none());
        let wheels = c.wheels.as_ref().unwrap();
        assert!(wheels[0].position_m.is_none() && wheels[0].radius_m == Some(0.5));
        assert!(wheels[1].radius_m.is_none() && wheels[1].position_m.is_some());
        assert_eq!(wheels[1].powered, Some(true));
        assert!(dropped.contains(&"truckConfig.hookPositionM"));
        assert!(dropped.contains(&"truckConfig.wheels.positionM"));
        assert!(dropped.contains(&"truckConfig.wheels.radiusM"));

        let mut f = frame();
        f.truck_config = Some(TruckConfig {
            wheels: Some(vec![WheelConfig::default(); MAX_WHEELS + 1]),
            ..TruckConfig::default()
        });
        assert!(f.sanitize().contains(&"truckConfig.wheels"));
        assert!(f.truck_config.unwrap().wheels.is_none());
    }

    #[test]
    fn serializes_without_absent_fields() {
        let json = serde_json::to_string(&frame()).unwrap();
        assert_eq!(
            json,
            r#"{"protocol":"roadpilot.telemetry","schemaVersion":2,"session":9,"sequence":1,"game":"ets2","pluginState":"running"}"#
        );
    }
}
