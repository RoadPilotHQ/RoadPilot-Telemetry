// Unit tests for the schema 2 serializer. They do not need the SCS SDK or a running game.
// The expected output is the shared fixture in schemas/fixtures, which the Rust client
// decodes in its own tests, so plugin and reader cannot drift apart unnoticed.

#include <clocale>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <limits>
#include <sstream>
#include <string>

#include "frame_v2.hpp"

using namespace roadpilot;

namespace {

int g_failures = 0;

void check(bool condition, const char* what) {
    if (!condition) {
        std::printf("FAILED: %s\n", what);
        ++g_failures;
    }
}

void check_equal(const std::string& actual, const std::string& expected, const char* what) {
    if (actual != expected) {
        std::printf("FAILED: %s\n  expected: %s\n  actual:   %s\n", what, expected.c_str(), actual.c_str());
        ++g_failures;
    }
}

std::string read_fixture(const char* name) {
    std::ifstream file(std::string(ROADPILOT_FIXTURES_DIR) + "/" + name, std::ios::binary);
    std::ostringstream content;
    content << file.rdbuf();
    return content.str();
}

// Removes whitespace outside of strings, turning a pretty-printed fixture into compact JSON.
std::string minify(const std::string& json) {
    std::string out;
    bool in_string = false;
    bool escaped = false;
    for (const char c : json) {
        if (in_string) {
            out.push_back(c);
            if (escaped) escaped = false;
            else if (c == '\\') escaped = true;
            else if (c == '"') in_string = false;
        } else if (c == '"') {
            in_string = true;
            out.push_back(c);
        } else if (c != ' ' && c != '\n' && c != '\r' && c != '\t') {
            out.push_back(c);
        }
    }
    return out;
}

std::string serialize(const FrameHeader& h, const DynamicState& d, const StaticState& s) {
    std::string out;
    serialize_frame_v2(h, d, s, out);
    return out;
}

void full_frame_matches_shared_fixture() {
    FrameHeader h;
    h.session = 4242424242ULL;
    h.sequence = 1234;
    h.game = "ets2";
    h.game_version = "1.21";

    DynamicState d;
    d.running = true;
    d.simulation_time_us.set(98765432);
    d.game_time_min.set(2000);
    d.local_scale.set(19.0f);
    TruckState& t = d.truck;
    t.speed_mps.set(22.5f);
    t.engine_rpm.set(1350.5f);
    t.gear.set(11);
    t.displayed_gear.set(11);
    t.engine_enabled.set(true);
    t.electric_enabled.set(true);
    t.fuel_l.set(412.5f);
    t.fuel_range_km.set(1180.0f);
    t.fuel_consumption_l_per_km.set(0.35f);
    t.fuel_warning.set(false);
    t.adblue_l.set(60.25f);
    t.oil_temperature_c.set(95.5f);
    t.water_temperature_c.set(88.0f);
    t.oil_pressure_psi.set(55.0f);
    t.battery_voltage_v.set(26.5f);
    t.brake_air_pressure_psi.set(120.0f);
    t.brake_temperature_c.set(45.0f);
    t.parking_brake.set(false);
    t.motor_brake.set(true);
    t.retarder_level.set(1);
    t.cruise_control_mps.set(22.222f);
    t.odometer_km.set(123456.7f);
    Inputs& i = d.inputs;
    i.steering_input.set(-0.125f);
    i.throttle_input.set(0.75f);
    i.brake_input.set(0.0f);
    i.clutch_input.set(0.0f);
    i.steering_effective.set(-0.1f);
    i.throttle_effective.set(0.7f);
    i.brake_effective.set(0.0f);
    i.clutch_effective.set(0.0f);
    d.motion.local_velocity_mps.set(Vec3f{0.25f, -0.5f, -22.5f});
    d.motion.local_acceleration_mps2.set(Vec3f{0.5f, 0.0f, -0.75f});
    d.motion.local_angular_velocity_rps.set(Vec3f{0.0f, 0.0125f, 0.0f});
    d.motion.wheel_steering_rot.set(-0.03f);
    d.placement.set(Placement{-31234.5678, 45.25, 12345.875, 0.25f, 0.01f, -0.005f});
    d.navigation.distance_m.set(742000.0f);
    d.navigation.time_s.set(33120.0f);
    d.navigation.speed_limit_mps.set(22.222f);
    d.cargo_damage.set(0.015f);

    StaticState s;
    s.truck_config.brand.set("Scania");
    s.truck_config.name.set("S");
    s.truck_config.fuel_capacity_l.set(800.0f);
    s.truck_config.adblue_capacity_l.set(80.0f);
    s.truck_config.rpm_limit.set(2500.0f);
    s.truck_config.forward_gears.set(12);
    s.truck_config.reverse_gears.set(2);
    s.truck_config.retarder_steps.set(3);
    Job& j = s.job;
    j.state = JobState::active;
    j.cargo.set("Electronics");
    j.cargo_mass_kg.set(12400.0f);
    j.cargo_loaded.set(true);
    j.source_city.set("Hamburg");
    j.source_company.set("Tradeaux");
    j.destination_city.set("Praha \"Hlavn\xC3\xAD\"");  // UTF-8 for "í"
    j.destination_company.set("Kaarfor");
    j.income.set(24360);
    j.delivery_time_min.set(3412);
    j.planned_distance_km.set(742);
    j.job_market.set("freight_market");

    check_equal(serialize(h, d, s), minify(read_fixture("frame-v2-full.json")), "full fixture");
}

void minimal_frame_matches_shared_fixture() {
    FrameHeader h;
    h.session = 1;
    h.sequence = 7;
    h.game = "ats";
    DynamicState d;  // paused, nothing known yet
    StaticState s;   // job state unknown -> no job object
    check_equal(serialize(h, d, s), minify(read_fixture("frame-v2-minimal.json")), "minimal fixture");
}

void omits_non_finite_values_and_empty_sections() {
    FrameHeader h;
    h.session = 1;
    DynamicState d;
    d.truck.speed_mps.set(std::numeric_limits<float>::quiet_NaN());
    d.motion.local_velocity_mps.set(Vec3f{0.0f, std::numeric_limits<float>::infinity(), 0.0f});
    d.placement.set(Placement{std::nan(""), 0, 0, 0, 0, 0});
    d.truck.gear.set(0);
    const std::string json = serialize(h, d, StaticState{});
    check(json.find("speedMps") == std::string::npos, "NaN speed omitted");
    check(json.find("motion") == std::string::npos, "section with only invalid values omitted");
    check(json.find("placement") == std::string::npos, "non-finite placement omitted");
    check(json.find("\"truck\":{\"gear\":0}") != std::string::npos, "zero gear kept");
    check(json.find("inputs") == std::string::npos, "empty inputs omitted");
}

void escapes_strings() {
    FrameHeader h;
    h.session = 1;
    StaticState s;
    s.job.state = JobState::active;
    s.job.cargo.set("a\"b\\c\nd\x01");
    const std::string json = serialize(h, DynamicState{}, s);
    check(json.find(R"("cargo":"a\"b\\c\u000ad\u0001")") != std::string::npos, "string escaping");
}

void writes_large_integers_exactly() {
    FrameHeader h;
    h.session = kMaxSafeInteger;
    h.sequence = kMaxSafeInteger;
    const std::string json = serialize(h, DynamicState{}, StaticState{});
    check(json.find("\"session\":9007199254740991") != std::string::npos, "max safe session");
    check(json.find("\"sequence\":9007199254740991") != std::string::npos, "max safe sequence");
    check(kMaxSafeInteger == 9007199254740991ULL, "constant is not truncated");
}

void job_states() {
    FrameHeader h;
    h.session = 1;
    DynamicState d;
    d.cargo_damage.set(0.5f);
    StaticState s;
    s.job.state = JobState::none;
    check(serialize(h, d, s).find("\"job\":{\"state\":\"none\"}") != std::string::npos, "no job");
    s.job.state = JobState::delivered;
    s.job.delivered_revenue.set(-150);
    const std::string delivered = serialize(h, d, s);
    check(delivered.find("\"state\":\"delivered\"") != std::string::npos, "delivered state");
    check(delivered.find("\"deliveredRevenue\":-150") != std::string::npos, "signed revenue");
    check(delivered.find("cargoDamage") == std::string::npos, "live damage only for active jobs");
}

void numbers_ignore_the_c_locale() {
    // A game or another plugin may switch to a locale with a decimal comma.
    if (!std::setlocale(LC_ALL, "de-DE")) std::setlocale(LC_ALL, "German");
    FrameHeader h;
    h.session = 1;
    DynamicState d;
    d.truck.speed_mps.set(1.5f);
    check(serialize(h, d, StaticState{}).find("\"speedMps\":1.5") != std::string::npos, "decimal point");
    std::setlocale(LC_ALL, "C");
}

void reuses_the_output_buffer() {
    FrameHeader h;
    h.session = 1;
    std::string out = "previous content";
    serialize_frame_v2(h, DynamicState{}, StaticState{}, out);
    check(out.rfind("{\"protocol\"", 0) == 0, "buffer cleared before writing");
}

}  // namespace

int main() {
    full_frame_matches_shared_fixture();
    minimal_frame_matches_shared_fixture();
    omits_non_finite_values_and_empty_sections();
    escapes_strings();
    writes_large_integers_exactly();
    job_states();
    numbers_ignore_the_c_locale();
    reuses_the_output_buffer();
    if (g_failures == 0) std::printf("All frame_v2 tests passed.\n");
    return g_failures == 0 ? 0 : 1;
}
