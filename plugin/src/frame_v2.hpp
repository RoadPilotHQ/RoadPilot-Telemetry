// RoadPilot telemetry frame, schema 2: plain data model and JSON serializer.
//
// This file does not depend on the SCS SDK, so the serializer can be unit tested on its own.
// Field names, order and units must match docs/PROTOCOL-V2.md and the shared fixtures in
// schemas/fixtures; the Rust client decodes the same fixtures.
#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <string>

namespace roadpilot {

// Largest integer represented exactly by JavaScript (2^53 - 1). Session ids, sequence
// numbers and timestamps never exceed it. (The first plugin declared this as uint32_t,
// which truncated it.)
constexpr std::uint64_t kMaxSafeInteger = 9007199254740991ULL;
constexpr int kSchemaVersion = 2;

template <class T>
struct Opt {
    bool has = false;
    T value{};
    void set(const T& v) {
        has = true;
        value = v;
    }
    void clear() { has = false; }
};

struct Vec3f {
    float x = 0, y = 0, z = 0;
};

// World placement in SDK units: meters (X east, Y up, Z south) and unit-range rotations.
struct Placement {
    double x = 0, y = 0, z = 0;
    float heading = 0, pitch = 0, roll = 0;
};

struct TruckState {
    Opt<float> speed_mps, engine_rpm;
    Opt<std::int32_t> gear, displayed_gear;
    Opt<bool> engine_enabled, electric_enabled;
    Opt<float> fuel_l, fuel_range_km, fuel_consumption_l_per_km;
    Opt<bool> fuel_warning;
    Opt<float> adblue_l, oil_temperature_c, water_temperature_c, oil_pressure_psi;
    Opt<float> battery_voltage_v, brake_air_pressure_psi, brake_temperature_c;
    Opt<bool> parking_brake, motor_brake;
    Opt<std::uint32_t> retarder_level;
    Opt<float> cruise_control_mps, odometer_km;
};

struct Inputs {
    Opt<float> steering_input, throttle_input, brake_input, clutch_input;
    Opt<float> steering_effective, throttle_effective, brake_effective, clutch_effective;
};

struct Motion {
    Opt<Vec3f> local_velocity_mps, local_acceleration_mps2, local_angular_velocity_rps;
    Opt<float> wheel_steering_rot;
};

// Most wheels the plugin reports per truck (SDK indices >= this are ignored).
constexpr std::size_t kMaxWheelsReported = 14;

// Static wheel geometry from the truck configuration. Positions are raw SDK vehicle space
// (X right, Y up, Z backwards), in metres; no conversion is done here.
struct WheelConfig {
    Opt<Vec3f> position_m;
    Opt<float> radius_m;
    Opt<bool> steerable, powered;
};

struct TruckConfig {
    Opt<std::string> brand, name;
    Opt<float> fuel_capacity_l, adblue_capacity_l, rpm_limit;
    Opt<std::uint32_t> forward_gears, reverse_gears, retarder_steps;
    // Number of wheels reported by the game (may exceed kMaxWheelsReported) and the
    // trailer hook position in vehicle space.
    Opt<std::uint32_t> wheel_count;
    Opt<Vec3f> hook_position_m;
    std::array<WheelConfig, kMaxWheelsReported> wheels;
};

// `unknown` until the game delivered its first job configuration; not serialized.
enum class JobState { unknown, none, active, delivered, cancelled };

struct Job {
    JobState state = JobState::unknown;
    Opt<std::string> cargo;
    Opt<float> cargo_mass_kg;
    Opt<bool> cargo_loaded;
    Opt<std::string> source_city, source_company, destination_city, destination_company;
    Opt<std::uint64_t> income;
    Opt<std::uint32_t> delivery_time_min, planned_distance_km;
    Opt<std::string> job_market;
    Opt<std::int64_t> delivered_revenue, cancel_penalty;
};

struct Navigation {
    Opt<float> distance_m, time_s, speed_limit_mps;
};

// Values updated by channel callbacks every frame. Contains no heap memory, so copying it
// on every frame does not allocate.
struct DynamicState {
    bool running = false;
    Opt<std::uint64_t> simulation_time_us;
    Opt<std::uint32_t> game_time_min;
    Opt<float> local_scale;
    TruckState truck;
    Inputs inputs;
    Motion motion;
    Opt<Placement> placement;
    Navigation navigation;
    // Channel `job.cargo.damage`; serialized inside the job object.
    Opt<float> cargo_damage;
};

// Values from configuration and gameplay events. Contains strings; copied only when changed.
struct StaticState {
    TruckConfig truck_config;
    Job job;
};

struct FrameHeader {
    std::uint64_t session = 0;
    std::uint64_t sequence = 0;
    const char* game = "ets2";  // "ets2" or "ats"
    std::string game_version;   // SDK telemetry game version "major.minor"; empty = absent
};

// Writes one compact JSON frame into `out` (cleared first; its capacity is reused).
// Absent values and sections without any value are omitted. Non-finite numbers are treated
// as absent because JSON cannot represent them.
void serialize_frame_v2(const FrameHeader& header, const DynamicState& dynamic,
                        const StaticState& stat, std::string& out);

}  // namespace roadpilot
