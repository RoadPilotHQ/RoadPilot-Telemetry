#include "frame_v2.hpp"

#include <charconv>
#include <cmath>
#include <cstddef>
#include <type_traits>

namespace roadpilot {
namespace {

// Minimal JSON writer. Objects that end up without any field are rolled back, so empty
// sections never appear on the wire. Numbers use std::to_chars: locale-independent and
// shortest round-trip representation.
class Writer {
public:
    explicit Writer(std::string& out) : out_(out) {}

    void begin_root() {
        out_.push_back('{');
        stack_[depth_++] = Level{out_.size() - 1, 0, true};
    }

    void end_root() { out_.push_back('}'); }

    void begin_object(const char* key) {
        const std::size_t start = out_.size();
        const std::size_t parent_fields = stack_[depth_ - 1].fields;
        key_prefix(key);
        out_.push_back('{');
        // Remember where to roll back and how many fields the parent had before.
        stack_[depth_++] = Level{start, parent_fields, false};
        stack_[depth_ - 1].fields = 0;
    }

    void end_object() {
        const Level level = stack_[--depth_];
        if (level.fields == 0) {
            out_.resize(level.start);
            stack_[depth_ - 1].fields = level.parent_fields;
        } else {
            out_.push_back('}');
        }
    }

    void string(const char* key, const std::string& value) {
        key_prefix(key);
        out_.push_back('"');
        for (const char ch : value) {
            const auto c = static_cast<unsigned char>(ch);
            if (c == '"' || c == '\\') {
                out_.push_back('\\');
                out_.push_back(ch);
            } else if (c < 0x20) {
                static const char hex[] = "0123456789abcdef";
                out_ += "\\u00";
                out_.push_back(hex[c >> 4]);
                out_.push_back(hex[c & 0xF]);
            } else {
                out_.push_back(ch);  // UTF-8 bytes pass through unchanged.
            }
        }
        out_.push_back('"');
    }

    void raw(const char* key, const char* literal) {
        key_prefix(key);
        out_ += literal;
    }

    template <class T>
    void number(const char* key, T value) {
        if constexpr (std::is_floating_point_v<T>) {
            if (!std::isfinite(value)) return;
        }
        key_prefix(key);
        char buffer[32];
        const auto result = std::to_chars(buffer, buffer + sizeof(buffer), value);
        out_.append(buffer, result.ptr);
    }

    void boolean(const char* key, bool value) { raw(key, value ? "true" : "false"); }

    template <class T>
    void opt_number(const char* key, const Opt<T>& v) {
        if (v.has) number(key, v.value);
    }
    void opt_bool(const char* key, const Opt<bool>& v) {
        if (v.has) boolean(key, v.value);
    }
    void opt_string(const char* key, const Opt<std::string>& v) {
        if (v.has) string(key, v.value);
    }

    void opt_vec3(const char* key, const Opt<Vec3f>& v) {
        if (!v.has) return;
        const Vec3f& p = v.value;
        if (!std::isfinite(p.x) || !std::isfinite(p.y) || !std::isfinite(p.z)) return;
        begin_object(key);
        number("x", p.x);
        number("y", p.y);
        number("z", p.z);
        end_object();
    }

private:
    struct Level {
        std::size_t start = 0;
        std::size_t parent_fields = 0;
        bool root = false;
        std::size_t fields = 0;
        Level() = default;
        Level(std::size_t s, std::size_t p, bool r) : start(s), parent_fields(p), root(r) {}
    };

    void key_prefix(const char* key) {
        Level& level = stack_[depth_ - 1];
        if (level.fields++ > 0) out_.push_back(',');
        out_.push_back('"');
        out_ += key;
        out_ += "\":";
    }

    std::string& out_;
    Level stack_[8];
    int depth_ = 0;
};

const char* job_state_name(JobState state) {
    switch (state) {
        case JobState::none: return "\"none\"";
        case JobState::active: return "\"active\"";
        case JobState::delivered: return "\"delivered\"";
        case JobState::cancelled: return "\"cancelled\"";
        case JobState::unknown: break;
    }
    return nullptr;
}

}  // namespace

void serialize_frame_v2(const FrameHeader& header, const DynamicState& d, const StaticState& s,
                        std::string& out) {
    out.clear();
    Writer w(out);
    w.begin_root();
    w.raw("protocol", "\"roadpilot.telemetry\"");
    w.number("schemaVersion", kSchemaVersion);
    w.number("session", header.session);
    w.number("sequence", header.sequence);
    w.raw("game", header.game[0] == 'a' ? "\"ats\"" : "\"ets2\"");
    if (!header.game_version.empty()) w.string("gameVersion", header.game_version);
    w.raw("pluginState", d.running ? "\"running\"" : "\"paused\"");
    w.opt_number("simulationTimeUs", d.simulation_time_us);
    w.opt_number("gameTimeMin", d.game_time_min);
    w.opt_number("localScale", d.local_scale);

    const TruckState& t = d.truck;
    w.begin_object("truck");
    w.opt_number("speedMps", t.speed_mps);
    w.opt_number("engineRpm", t.engine_rpm);
    w.opt_number("gear", t.gear);
    w.opt_number("displayedGear", t.displayed_gear);
    w.opt_bool("engineEnabled", t.engine_enabled);
    w.opt_bool("electricEnabled", t.electric_enabled);
    w.opt_number("fuelL", t.fuel_l);
    w.opt_number("fuelRangeKm", t.fuel_range_km);
    w.opt_number("fuelConsumptionLPerKm", t.fuel_consumption_l_per_km);
    w.opt_bool("fuelWarning", t.fuel_warning);
    w.opt_number("adblueL", t.adblue_l);
    w.opt_number("oilTemperatureC", t.oil_temperature_c);
    w.opt_number("waterTemperatureC", t.water_temperature_c);
    w.opt_number("oilPressurePsi", t.oil_pressure_psi);
    w.opt_number("batteryVoltageV", t.battery_voltage_v);
    w.opt_number("brakeAirPressurePsi", t.brake_air_pressure_psi);
    w.opt_number("brakeTemperatureC", t.brake_temperature_c);
    w.opt_bool("parkingBrake", t.parking_brake);
    w.opt_bool("motorBrake", t.motor_brake);
    w.opt_number("retarderLevel", t.retarder_level);
    w.opt_number("cruiseControlMps", t.cruise_control_mps);
    w.opt_number("odometerKm", t.odometer_km);
    w.end_object();

    const Inputs& i = d.inputs;
    w.begin_object("inputs");
    w.opt_number("steeringInput", i.steering_input);
    w.opt_number("throttleInput", i.throttle_input);
    w.opt_number("brakeInput", i.brake_input);
    w.opt_number("clutchInput", i.clutch_input);
    w.opt_number("steeringEffective", i.steering_effective);
    w.opt_number("throttleEffective", i.throttle_effective);
    w.opt_number("brakeEffective", i.brake_effective);
    w.opt_number("clutchEffective", i.clutch_effective);
    w.end_object();

    w.begin_object("motion");
    w.opt_vec3("localVelocityMps", d.motion.local_velocity_mps);
    w.opt_vec3("localAccelerationMps2", d.motion.local_acceleration_mps2);
    w.opt_vec3("localAngularVelocityRps", d.motion.local_angular_velocity_rps);
    w.opt_number("wheelSteeringRot", d.motion.wheel_steering_rot);
    w.end_object();

    if (d.placement.has) {
        const Placement& p = d.placement.value;
        const bool finite = std::isfinite(p.x) && std::isfinite(p.y) && std::isfinite(p.z) &&
                            std::isfinite(p.heading) && std::isfinite(p.pitch) &&
                            std::isfinite(p.roll);
        if (finite) {
            w.begin_object("placement");
            w.number("x", p.x);
            w.number("y", p.y);
            w.number("z", p.z);
            w.number("heading", p.heading);
            w.number("pitch", p.pitch);
            w.number("roll", p.roll);
            w.end_object();
        }
    }

    const TruckConfig& c = s.truck_config;
    w.begin_object("truckConfig");
    w.opt_string("brand", c.brand);
    w.opt_string("name", c.name);
    w.opt_number("fuelCapacityL", c.fuel_capacity_l);
    w.opt_number("adblueCapacityL", c.adblue_capacity_l);
    w.opt_number("rpmLimit", c.rpm_limit);
    w.opt_number("forwardGears", c.forward_gears);
    w.opt_number("reverseGears", c.reverse_gears);
    w.opt_number("retarderSteps", c.retarder_steps);
    w.end_object();

    const Job& j = s.job;
    if (const char* state = job_state_name(j.state)) {
        w.begin_object("job");
        w.raw("state", state);
        w.opt_string("cargo", j.cargo);
        w.opt_number("cargoMassKg", j.cargo_mass_kg);
        if (j.state == JobState::active) w.opt_number("cargoDamage", d.cargo_damage);
        w.opt_bool("cargoLoaded", j.cargo_loaded);
        w.opt_string("sourceCity", j.source_city);
        w.opt_string("sourceCompany", j.source_company);
        w.opt_string("destinationCity", j.destination_city);
        w.opt_string("destinationCompany", j.destination_company);
        w.opt_number("income", j.income);
        w.opt_number("deliveryTimeMin", j.delivery_time_min);
        w.opt_number("plannedDistanceKm", j.planned_distance_km);
        w.opt_string("jobMarket", j.job_market);
        w.opt_number("deliveredRevenue", j.delivered_revenue);
        w.opt_number("cancelPenalty", j.cancel_penalty);
        w.end_object();
    }

    w.begin_object("navigation");
    w.opt_number("distanceM", d.navigation.distance_m);
    w.opt_number("timeS", d.navigation.time_s);
    w.opt_number("speedLimitMps", d.navigation.speed_limit_mps);
    w.end_object();

    w.end_root();
}

}  // namespace roadpilot
