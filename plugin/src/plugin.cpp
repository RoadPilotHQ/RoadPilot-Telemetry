// RoadPilot SCS telemetry plugin for ETS2 and ATS (Windows).
//
// Threading: the SCS SDK calls every callback on the game's main thread. Channel callbacks
// only write into `g_stage`, which is owned by that thread and needs no lock. On every
// FRAME_END the stage is copied once into `g_shared` under a mutex, and a worker thread
// serializes the newest frame and writes it to the RoadPilot named pipe. Pipe I/O never
// happens on the game thread, so a slow or missing reader cannot stall the game.

#include <windows.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>

#include "scssdk.h"
#include "scssdk_telemetry.h"
#include "common/scssdk_telemetry_common_channels.h"
#include "common/scssdk_telemetry_common_configs.h"
#include "common/scssdk_telemetry_common_gameplay_events.h"
#include "common/scssdk_telemetry_job_common_channels.h"
#include "common/scssdk_telemetry_truck_common_channels.h"
#include "eurotrucks2/scssdk_eut2.h"
#include "amtrucks/scssdk_ats.h"

#include "frame_v2.hpp"

using namespace roadpilot;

namespace {

constexpr wchar_t kPipeName[] = L"\\\\.\\pipe\\RoadPilot.Telemetry.v1";
// SDK 1.11+: up to 14 wheels expose indexed channels.
constexpr scs_u32_t kMaxWheels = 14;
static_assert(kMaxWheels == kMaxWheelsReported, "wheel limits must match the frame model");
// At most ~60 frames per second are written; newer frames replace unsent ones.
constexpr auto kMinSendInterval = std::chrono::milliseconds(16);
constexpr auto kReconnectDelay = std::chrono::milliseconds(1000);
constexpr auto kWriteFailureDelay = std::chrono::milliseconds(250);

// ---------------------------------------------------------------------------------------
// Game-thread state. Only touched from SDK callbacks.

struct Stage {
    DynamicState dynamic;
    StaticState stat;
    std::uint64_t static_version = 0;
    Opt<float> wheel_steering[kMaxWheels];
    bool wheel_steerable[kMaxWheels] = {};
};

Stage g_stage;
FrameHeader g_header;
scs_log_t g_log = nullptr;

// ---------------------------------------------------------------------------------------
// Shared between the game thread and the worker. Guarded by g_mutex.

struct Shared {
    DynamicState dynamic;
    StaticState stat;
    std::uint64_t static_version = 0;
    std::uint64_t sequence = 0;
    bool dirty = false;
    bool stopping = false;
};

std::mutex g_mutex;
std::condition_variable g_changed;
Shared g_shared;
std::thread g_worker;
HANDLE g_worker_handle = nullptr;

// Transport status, reported to the game log on change (from the game thread).
enum TransportState : int { kStarting = 0, kReaderUnavailable, kConnected, kWriteFailed };
std::atomic<int> g_transport_state{kStarting};
std::atomic<DWORD> g_transport_error{ERROR_SUCCESS};
int g_reported_transport_state = -1;

void log_message(scs_log_type_t type, const char* message) {
    if (g_log) g_log(type, message);
}

void report_transport_status() {
    const int state = g_transport_state.load(std::memory_order_relaxed);
    if (state == kStarting || state == g_reported_transport_state) return;
    g_reported_transport_state = state;
    char message[192];
    const auto error = static_cast<unsigned long>(g_transport_error.load(std::memory_order_relaxed));
    switch (state) {
        case kReaderUnavailable:
            std::snprintf(message, sizeof(message),
                          "RoadPilot: desktop reader not available (Windows error %lu); retrying.", error);
            break;
        case kConnected:
            std::snprintf(message, sizeof(message), "RoadPilot: connected to the desktop reader.");
            break;
        default:
            std::snprintf(message, sizeof(message),
                          "RoadPilot: telemetry pipe write failed (Windows error %lu); reconnecting.",
                          error);
            break;
    }
    log_message(state == kConnected ? SCS_LOG_TYPE_message : SCS_LOG_TYPE_warning, message);
}

std::uint64_t make_session_id() {
    LARGE_INTEGER counter{};
    QueryPerformanceCounter(&counter);
    std::uint64_t x = static_cast<std::uint64_t>(counter.QuadPart) ^
                      (static_cast<std::uint64_t>(GetCurrentProcessId()) << 32) ^ GetTickCount64();
    // splitmix64 finalizer to spread the bits, then keep it JavaScript-safe and non-zero.
    x += 0x9E3779B97F4A7C15ULL;
    x = (x ^ (x >> 30)) * 0xBF58476D1CE4E5B9ULL;
    x = (x ^ (x >> 27)) * 0x94D049BB133111EBULL;
    x ^= x >> 31;
    x &= kMaxSafeInteger;
    return x == 0 ? 1 : x;
}

// ---------------------------------------------------------------------------------------
// Transport worker: one persistent pipe connection, reconnect on failure.

bool write_all(HANDLE pipe, const void* data, DWORD size, DWORD* failure_error) {
    const auto* bytes = static_cast<const unsigned char*>(data);
    DWORD offset = 0;
    while (offset < size) {
        DWORD written = 0;
        if (!WriteFile(pipe, bytes + offset, size - offset, &written, nullptr)) {
            *failure_error = GetLastError();
            return false;
        }
        if (written == 0) {
            *failure_error = ERROR_WRITE_FAULT;
            return false;
        }
        offset += written;
    }
    return true;
}

// Waits up to `delay` unless the plugin is stopping. Returns false when stopping.
bool wait_unless_stopping(std::unique_lock<std::mutex>& lock, std::chrono::milliseconds delay) {
    return !g_changed.wait_for(lock, delay, [] { return g_shared.stopping; });
}

void transport_worker() {
    HANDLE pipe = INVALID_HANDLE_VALUE;
    DynamicState dynamic;
    StaticState stat;
    std::uint64_t static_version = ~0ULL;
    FrameHeader header = g_header;
    std::string payload;
    payload.reserve(4096);
    auto last_send = std::chrono::steady_clock::now() - kMinSendInterval;

    for (;;) {
        {
            std::unique_lock<std::mutex> lock(g_mutex);
            g_changed.wait(lock, [] { return g_shared.stopping || g_shared.dirty; });
            if (g_shared.stopping) break;
            const auto next_allowed = last_send + kMinSendInterval;
            if (std::chrono::steady_clock::now() < next_allowed) {
                g_changed.wait_until(lock, next_allowed, [] { return g_shared.stopping; });
                if (g_shared.stopping) break;
            }
            g_shared.dirty = false;
            dynamic = g_shared.dynamic;
            if (g_shared.static_version != static_version) {
                stat = g_shared.stat;  // Strings are copied only when they changed.
                static_version = g_shared.static_version;
            }
            header.sequence = g_shared.sequence;
        }
        last_send = std::chrono::steady_clock::now();

        if (pipe == INVALID_HANDLE_VALUE) {
            pipe = CreateFileW(kPipeName, GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
            if (pipe == INVALID_HANDLE_VALUE) {
                g_transport_error.store(GetLastError(), std::memory_order_relaxed);
                g_transport_state.store(kReaderUnavailable, std::memory_order_relaxed);
                std::unique_lock<std::mutex> lock(g_mutex);
                if (!wait_unless_stopping(lock, kReconnectDelay)) break;
                continue;
            }
        }
        serialize_frame_v2(header, dynamic, stat, payload);

        const auto length = static_cast<std::uint32_t>(payload.size());
        const unsigned char prefix[4] = {
            static_cast<unsigned char>(length), static_cast<unsigned char>(length >> 8),
            static_cast<unsigned char>(length >> 16), static_cast<unsigned char>(length >> 24)};
        DWORD error = ERROR_SUCCESS;
        if (write_all(pipe, prefix, sizeof(prefix), &error) &&
            write_all(pipe, payload.data(), length, &error)) {
            g_transport_state.store(kConnected, std::memory_order_relaxed);
            continue;
        }

        // The reader went away or the pipe broke: drop the handle and reconnect later.
        CloseHandle(pipe);
        pipe = INVALID_HANDLE_VALUE;
        g_transport_error.store(error, std::memory_order_relaxed);
        g_transport_state.store(kWriteFailed, std::memory_order_relaxed);
        std::unique_lock<std::mutex> lock(g_mutex);
        if (!wait_unless_stopping(lock, kWriteFailureDelay)) break;
    }

    if (pipe != INVALID_HANDLE_VALUE) CloseHandle(pipe);
}

void start_worker() {
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_shared = Shared{};
    }
    g_worker = std::thread(transport_worker);
    g_worker_handle = static_cast<HANDLE>(g_worker.native_handle());
}

void stop_worker() {
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_shared.stopping = true;
    }
    g_changed.notify_all();
    // Unblocks a WriteFile that waits for a reader which stopped reading.
    if (g_worker_handle) CancelSynchronousIo(g_worker_handle);
    if (g_worker.joinable()) g_worker.join();
    g_worker_handle = nullptr;
}

// ---------------------------------------------------------------------------------------
// Channel callbacks. `context` points at the destination inside g_stage. A NULL value
// means the game currently has no value for the channel.

SCSAPI_VOID on_float(const scs_string_t, const scs_u32_t, const scs_value_t* value,
                     const scs_context_t context) {
    auto* target = static_cast<Opt<float>*>(context);
    if (value) target->set(value->value_float.value);
    else target->clear();
}

SCSAPI_VOID on_bool(const scs_string_t, const scs_u32_t, const scs_value_t* value,
                    const scs_context_t context) {
    auto* target = static_cast<Opt<bool>*>(context);
    if (value) target->set(value->value_bool.value != 0);
    else target->clear();
}

SCSAPI_VOID on_s32(const scs_string_t, const scs_u32_t, const scs_value_t* value,
                   const scs_context_t context) {
    auto* target = static_cast<Opt<std::int32_t>*>(context);
    if (value) target->set(value->value_s32.value);
    else target->clear();
}

SCSAPI_VOID on_u32(const scs_string_t, const scs_u32_t, const scs_value_t* value,
                   const scs_context_t context) {
    auto* target = static_cast<Opt<std::uint32_t>*>(context);
    if (value) target->set(value->value_u32.value);
    else target->clear();
}

SCSAPI_VOID on_fvector(const scs_string_t, const scs_u32_t, const scs_value_t* value,
                       const scs_context_t context) {
    auto* target = static_cast<Opt<Vec3f>*>(context);
    if (value) target->set(Vec3f{value->value_fvector.x, value->value_fvector.y, value->value_fvector.z});
    else target->clear();
}

SCSAPI_VOID on_dplacement(const scs_string_t, const scs_u32_t, const scs_value_t* value,
                          const scs_context_t context) {
    auto* target = static_cast<Opt<Placement>*>(context);
    if (!value) {
        target->clear();
        return;
    }
    const scs_value_dplacement_t& p = value->value_dplacement;
    target->set(Placement{p.position.x, p.position.y, p.position.z, p.orientation.heading,
                          p.orientation.pitch, p.orientation.roll});
}

// ---------------------------------------------------------------------------------------
// Configuration and gameplay events.

void set_string(Opt<std::string>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_string && value.value_string.value) target.set(value.value_string.value);
}
void set_float(Opt<float>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_float) target.set(value.value_float.value);
}
void set_u32(Opt<std::uint32_t>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_u32) target.set(value.value_u32.value);
}
void set_u64(Opt<std::uint64_t>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_u64) target.set(value.value_u64.value);
    else if (value.type == SCS_VALUE_TYPE_u32) target.set(value.value_u32.value);
}
void set_s64(Opt<std::int64_t>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_s64) target.set(value.value_s64.value);
}
void set_fvector(Opt<Vec3f>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_fvector) {
        target.set(Vec3f{value.value_fvector.x, value.value_fvector.y, value.value_fvector.z});
    }
}
void set_bool(Opt<bool>& target, const scs_value_t& value) {
    if (value.type == SCS_VALUE_TYPE_bool) target.set(value.value_bool.value != 0);
}

bool is(const char* a, const char* b) { return std::strcmp(a, b) == 0; }

void apply_truck_config(const scs_named_value_t* attributes) {
    TruckConfig config;
    bool steerable[kMaxWheels] = {};
    for (const scs_named_value_t* a = attributes; a->name; ++a) {
        const scs_value_t& v = a->value;
        if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_brand)) set_string(config.brand, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_name)) set_string(config.name, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_fuel_capacity)) set_float(config.fuel_capacity_l, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_adblue_capacity)) set_float(config.adblue_capacity_l, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_rpm_limit)) set_float(config.rpm_limit, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_forward_gear_count)) set_u32(config.forward_gears, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_reverse_gear_count)) set_u32(config.reverse_gears, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_retarder_step_count)) set_u32(config.retarder_steps, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_wheel_count)) set_u32(config.wheel_count, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_hook_position)) set_fvector(config.hook_position_m, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_wheel_position) && a->index < kMaxWheels) {
            set_fvector(config.wheels[a->index].position_m, v);
        } else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_wheel_radius) && a->index < kMaxWheels) {
            set_float(config.wheels[a->index].radius_m, v);
        } else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_wheel_powered) && a->index < kMaxWheels) {
            set_bool(config.wheels[a->index].powered, v);
        } else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_wheel_steerable) && a->index < kMaxWheels &&
                   v.type == SCS_VALUE_TYPE_bool) {
            steerable[a->index] = v.value_bool.value != 0;
            config.wheels[a->index].steerable.set(steerable[a->index]);
        }
    }
    // An empty configuration means the truck was removed; all values become absent.
    g_stage.stat.truck_config = config;
    std::memcpy(g_stage.wheel_steerable, steerable, sizeof(steerable));
    ++g_stage.static_version;
}

void apply_job_config(const scs_named_value_t* attributes) {
    Job& current = g_stage.stat.job;
    if (!attributes->name) {
        // No job. A delivered or cancelled job stays visible until the next job starts.
        if (current.state != JobState::delivered && current.state != JobState::cancelled) {
            current = Job{};
            current.state = JobState::none;
        }
        ++g_stage.static_version;
        return;
    }
    Job job;
    job.state = JobState::active;
    for (const scs_named_value_t* a = attributes; a->name; ++a) {
        const scs_value_t& v = a->value;
        if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_cargo)) set_string(job.cargo, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_cargo_mass)) set_float(job.cargo_mass_kg, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_is_cargo_loaded)) set_bool(job.cargo_loaded, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_source_city)) set_string(job.source_city, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_source_company)) set_string(job.source_company, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_destination_city)) set_string(job.destination_city, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_destination_company)) set_string(job.destination_company, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_income)) set_u64(job.income, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_delivery_time)) set_u32(job.delivery_time_min, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_planned_distance_km)) set_u32(job.planned_distance_km, v);
        else if (is(a->name, SCS_TELEMETRY_CONFIG_ATTRIBUTE_job_market)) set_string(job.job_market, v);
    }
    current = job;
    ++g_stage.static_version;
}

SCSAPI_VOID on_configuration(const scs_event_t, const void* event_info, const scs_context_t) {
    const auto* info = static_cast<const scs_telemetry_configuration_t*>(event_info);
    if (!info || !info->id || !info->attributes) return;
    if (is(info->id, SCS_TELEMETRY_CONFIG_truck)) apply_truck_config(info->attributes);
    else if (is(info->id, SCS_TELEMETRY_CONFIG_job)) apply_job_config(info->attributes);
}

SCSAPI_VOID on_gameplay(const scs_event_t, const void* event_info, const scs_context_t) {
    const auto* info = static_cast<const scs_telemetry_gameplay_event_t*>(event_info);
    if (!info || !info->id || !info->attributes) return;
    Job& job = g_stage.stat.job;
    if (is(info->id, SCS_TELEMETRY_GAMEPLAY_EVENT_job_delivered)) {
        job.state = JobState::delivered;
        for (const scs_named_value_t* a = info->attributes; a->name; ++a) {
            if (is(a->name, SCS_TELEMETRY_GAMEPLAY_EVENT_ATTRIBUTE_revenue)) set_s64(job.delivered_revenue, a->value);
        }
        ++g_stage.static_version;
    } else if (is(info->id, SCS_TELEMETRY_GAMEPLAY_EVENT_job_cancelled)) {
        job.state = JobState::cancelled;
        for (const scs_named_value_t* a = info->attributes; a->name; ++a) {
            if (is(a->name, SCS_TELEMETRY_GAMEPLAY_EVENT_ATTRIBUTE_cancel_penalty)) set_s64(job.cancel_penalty, a->value);
        }
        ++g_stage.static_version;
    }
}

SCSAPI_VOID on_frame_start(const scs_event_t, const void* event_info, const scs_context_t) {
    const auto* info = static_cast<const scs_telemetry_frame_start_t*>(event_info);
    if (!info) return;
    if (info->simulation_time <= kMaxSafeInteger) g_stage.dynamic.simulation_time_us.set(info->simulation_time);
    else g_stage.dynamic.simulation_time_us.clear();
}

SCSAPI_VOID on_pause_change(const scs_event_t event, const void*, const scs_context_t) {
    g_stage.dynamic.running = event == SCS_TELEMETRY_EVENT_started;
}

SCSAPI_VOID on_frame_end(const scs_event_t, const void*, const scs_context_t) {
    // Physical steering angle of the first steerable wheel.
    Opt<float> steering;
    for (scs_u32_t i = 0; i < kMaxWheels; ++i) {
        if (g_stage.wheel_steerable[i] && g_stage.wheel_steering[i].has) {
            steering = g_stage.wheel_steering[i];
            break;
        }
    }
    g_stage.dynamic.motion.wheel_steering_rot = steering;

    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_shared.dynamic = g_stage.dynamic;
        if (g_shared.static_version != g_stage.static_version) {
            g_shared.stat = g_stage.stat;
            g_shared.static_version = g_stage.static_version;
        }
        if (g_shared.sequence < kMaxSafeInteger) ++g_shared.sequence;
        g_shared.dirty = true;
    }
    g_changed.notify_one();
    report_transport_status();
}

// ---------------------------------------------------------------------------------------
// Registration.

struct Registrar {
    const scs_telemetry_init_params_v100_t* params;
    int unavailable = 0;

    void channel(const char* name, scs_value_type_t type, scs_telemetry_channel_callback_t callback,
                 void* target, scs_u32_t index = SCS_U32_NIL) {
        if (params->register_for_channel(name, index, type, SCS_TELEMETRY_CHANNEL_FLAG_no_value,
                                         callback, target) != SCS_RESULT_ok) {
            ++unavailable;  // Not supported by this game/version; the value stays absent.
        }
    }
};

void register_channels(const scs_telemetry_init_params_v100_t* params) {
    Registrar r{params};
    DynamicState& d = g_stage.dynamic;
    TruckState& t = d.truck;
    Inputs& i = d.inputs;

    r.channel(SCS_TELEMETRY_CHANNEL_game_time, SCS_VALUE_TYPE_u32, on_u32, &d.game_time_min);
    r.channel(SCS_TELEMETRY_CHANNEL_local_scale, SCS_VALUE_TYPE_float, on_float, &d.local_scale);

    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_speed, SCS_VALUE_TYPE_float, on_float, &t.speed_mps);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_engine_rpm, SCS_VALUE_TYPE_float, on_float, &t.engine_rpm);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_engine_gear, SCS_VALUE_TYPE_s32, on_s32, &t.gear);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_displayed_gear, SCS_VALUE_TYPE_s32, on_s32, &t.displayed_gear);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_engine_enabled, SCS_VALUE_TYPE_bool, on_bool, &t.engine_enabled);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_electric_enabled, SCS_VALUE_TYPE_bool, on_bool, &t.electric_enabled);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_fuel, SCS_VALUE_TYPE_float, on_float, &t.fuel_l);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_fuel_range, SCS_VALUE_TYPE_float, on_float, &t.fuel_range_km);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_fuel_average_consumption, SCS_VALUE_TYPE_float, on_float,
              &t.fuel_consumption_l_per_km);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_fuel_warning, SCS_VALUE_TYPE_bool, on_bool, &t.fuel_warning);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_adblue, SCS_VALUE_TYPE_float, on_float, &t.adblue_l);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_oil_temperature, SCS_VALUE_TYPE_float, on_float, &t.oil_temperature_c);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_water_temperature, SCS_VALUE_TYPE_float, on_float,
              &t.water_temperature_c);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_oil_pressure, SCS_VALUE_TYPE_float, on_float, &t.oil_pressure_psi);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_battery_voltage, SCS_VALUE_TYPE_float, on_float, &t.battery_voltage_v);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_brake_air_pressure, SCS_VALUE_TYPE_float, on_float,
              &t.brake_air_pressure_psi);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_brake_temperature, SCS_VALUE_TYPE_float, on_float,
              &t.brake_temperature_c);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_parking_brake, SCS_VALUE_TYPE_bool, on_bool, &t.parking_brake);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_motor_brake, SCS_VALUE_TYPE_bool, on_bool, &t.motor_brake);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_retarder_level, SCS_VALUE_TYPE_u32, on_u32, &t.retarder_level);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_cruise_control, SCS_VALUE_TYPE_float, on_float, &t.cruise_control_mps);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_odometer, SCS_VALUE_TYPE_float, on_float, &t.odometer_km);

    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_input_steering, SCS_VALUE_TYPE_float, on_float, &i.steering_input);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_input_throttle, SCS_VALUE_TYPE_float, on_float, &i.throttle_input);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_input_brake, SCS_VALUE_TYPE_float, on_float, &i.brake_input);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_input_clutch, SCS_VALUE_TYPE_float, on_float, &i.clutch_input);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_effective_steering, SCS_VALUE_TYPE_float, on_float,
              &i.steering_effective);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_effective_throttle, SCS_VALUE_TYPE_float, on_float,
              &i.throttle_effective);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_effective_brake, SCS_VALUE_TYPE_float, on_float, &i.brake_effective);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_effective_clutch, SCS_VALUE_TYPE_float, on_float, &i.clutch_effective);

    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_local_linear_velocity, SCS_VALUE_TYPE_fvector, on_fvector,
              &d.motion.local_velocity_mps);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_local_linear_acceleration, SCS_VALUE_TYPE_fvector, on_fvector,
              &d.motion.local_acceleration_mps2);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_local_angular_velocity, SCS_VALUE_TYPE_fvector, on_fvector,
              &d.motion.local_angular_velocity_rps);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_world_placement, SCS_VALUE_TYPE_dplacement, on_dplacement,
              &d.placement);

    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_navigation_distance, SCS_VALUE_TYPE_float, on_float,
              &d.navigation.distance_m);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_navigation_time, SCS_VALUE_TYPE_float, on_float, &d.navigation.time_s);
    r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_navigation_speed_limit, SCS_VALUE_TYPE_float, on_float,
              &d.navigation.speed_limit_mps);

    r.channel(SCS_TELEMETRY_JOB_CHANNEL_cargo_damage, SCS_VALUE_TYPE_float, on_float, &d.cargo_damage);

    const int scalar_unavailable = r.unavailable;
    for (scs_u32_t wheel = 0; wheel < kMaxWheels; ++wheel) {
        r.channel(SCS_TELEMETRY_TRUCK_CHANNEL_wheel_steering, SCS_VALUE_TYPE_float, on_float,
                  &g_stage.wheel_steering[wheel], wheel);
    }

    if (scalar_unavailable > 0) {
        char message[128];
        std::snprintf(message, sizeof(message),
                      "RoadPilot: %d telemetry channels are not provided by this game version.",
                      scalar_unavailable);
        log_message(SCS_LOG_TYPE_message, message);
    }
}

}  // namespace

SCSAPI_RESULT scs_telemetry_init(const scs_u32_t version, const scs_telemetry_init_params_t* params) {
    if (version != SCS_TELEMETRY_VERSION_1_00 && version != SCS_TELEMETRY_VERSION_1_01) {
        return SCS_RESULT_unsupported;
    }
    const auto* p = static_cast<const scs_telemetry_init_params_v100_t*>(params);
    if (!p || !p->common.game_id) return SCS_RESULT_invalid_parameter;
    const char* game = nullptr;
    if (std::strcmp(p->common.game_id, SCS_GAME_ID_EUT2) == 0) game = "ets2";
    else if (std::strcmp(p->common.game_id, SCS_GAME_ID_ATS) == 0) game = "ats";
    else return SCS_RESULT_unsupported;

    g_log = p->common.log;
    g_stage = Stage{};
    g_header = FrameHeader{};
    g_header.game = game;
    g_header.session = make_session_id();
    char game_version[16];
    std::snprintf(game_version, sizeof(game_version), "%u.%u",
                  static_cast<unsigned>(SCS_GET_MAJOR_VERSION(p->common.game_version)),
                  static_cast<unsigned>(SCS_GET_MINOR_VERSION(p->common.game_version)));
    g_header.game_version = game_version;
    g_transport_state.store(kStarting, std::memory_order_relaxed);
    g_transport_error.store(ERROR_SUCCESS, std::memory_order_relaxed);
    g_reported_transport_state = -1;

    // FRAME_END and pause state are required; the rest only adds information.
    if (p->register_for_event(SCS_TELEMETRY_EVENT_frame_end, on_frame_end, nullptr) != SCS_RESULT_ok ||
        p->register_for_event(SCS_TELEMETRY_EVENT_started, on_pause_change, nullptr) != SCS_RESULT_ok ||
        p->register_for_event(SCS_TELEMETRY_EVENT_paused, on_pause_change, nullptr) != SCS_RESULT_ok) {
        log_message(SCS_LOG_TYPE_error, "RoadPilot: required telemetry events are not available.");
        g_log = nullptr;
        return SCS_RESULT_generic_error;
    }
    p->register_for_event(SCS_TELEMETRY_EVENT_frame_start, on_frame_start, nullptr);
    p->register_for_event(SCS_TELEMETRY_EVENT_configuration, on_configuration, nullptr);
    p->register_for_event(SCS_TELEMETRY_EVENT_gameplay, on_gameplay, nullptr);
    register_channels(p);

    start_worker();
    log_message(SCS_LOG_TYPE_message, "RoadPilot telemetry plugin initialized (schema 2).");
    return SCS_RESULT_ok;
}

SCSAPI_VOID scs_telemetry_shutdown(void) {
    // The game removes all registrations itself; only the worker has to stop.
    stop_worker();
    g_stage = Stage{};
    g_log = nullptr;
}
