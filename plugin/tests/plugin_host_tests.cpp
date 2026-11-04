// Integration test of the built plugin DLL without a game.
//
// The test acts as a minimal SCS host: it loads roadpilot-telemetry.dll, calls
// scs_telemetry_init with its own registration functions, plays configuration, gameplay,
// channel and frame events, and reads the frames from the real RoadPilot pipe.
//
// It uses the real pipe name \\.\pipe\RoadPilot.Telemetry.v1, so RoadPilot and the console
// reader must not be running while it executes.

#include <windows.h>

#include <chrono>
#include <cstdio>
#include <cstring>
#include <map>
#include <string>
#include <thread>
#include <vector>

#include "scssdk.h"
#include "scssdk_telemetry.h"
#include "common/scssdk_telemetry_common_configs.h"
#include "common/scssdk_telemetry_common_gameplay_events.h"
#include "common/scssdk_telemetry_truck_common_channels.h"
#include "eurotrucks2/scssdk_eut2.h"

namespace {

int g_failures = 0;

void check(bool condition, const char* what) {
    if (!condition) {
        std::printf("FAILED: %s\n", what);
        ++g_failures;
    }
}

// ---------------------------------------------------------------- fake SCS host

struct Channel {
    scs_u32_t index;
    scs_telemetry_channel_callback_t callback;
    scs_context_t context;
};

std::map<scs_event_t, std::pair<scs_telemetry_event_callback_t, scs_context_t>> g_events;
std::multimap<std::string, Channel> g_channels;

SCSAPI_RESULT register_event(const scs_event_t event, const scs_telemetry_event_callback_t callback,
                             const scs_context_t context) {
    g_events[event] = {callback, context};
    return SCS_RESULT_ok;
}
SCSAPI_RESULT unregister_event(const scs_event_t event) {
    g_events.erase(event);
    return SCS_RESULT_ok;
}
SCSAPI_RESULT register_channel(const scs_string_t name, const scs_u32_t index, const scs_value_type_t,
                               const scs_u32_t, const scs_telemetry_channel_callback_t callback,
                               const scs_context_t context) {
    // Simulate a game that does not provide AdBlue (like ATS).
    if (std::strcmp(name, SCS_TELEMETRY_TRUCK_CHANNEL_adblue) == 0) return SCS_RESULT_not_found;
    g_channels.emplace(name, Channel{index, callback, context});
    return SCS_RESULT_ok;
}
SCSAPI_RESULT unregister_channel(const scs_string_t, const scs_u32_t, const scs_value_type_t) {
    return SCS_RESULT_ok;
}
SCSAPI_VOID log_line(const scs_log_type_t, const scs_string_t message) {
    std::printf("  [game log] %s\n", message);
}

void fire(scs_event_t event, const void* info) {
    const auto it = g_events.find(event);
    if (it != g_events.end()) it->second.first(event, info, it->second.second);
}

void channel(const char* name, const scs_value_t& value, scs_u32_t index = SCS_U32_NIL) {
    const auto range = g_channels.equal_range(name);
    for (auto it = range.first; it != range.second; ++it) {
        if (it->second.index == index) it->second.callback(name, index, &value, it->second.context);
    }
}

scs_value_t float_value(float v) {
    scs_value_t value{};
    value.type = SCS_VALUE_TYPE_float;
    value.value_float.value = v;
    return value;
}

scs_named_value_t named(const char* name, scs_value_t value, scs_u32_t index = SCS_U32_NIL) {
    scs_named_value_t n{};
    n.name = name;
    n.index = index;
    n.value = value;
    return n;
}

void configuration(const char* id, std::vector<scs_named_value_t> attributes) {
    attributes.push_back(scs_named_value_t{});  // terminator
    scs_telemetry_configuration_t info{id, attributes.data()};
    fire(SCS_TELEMETRY_EVENT_configuration, &info);
}

void play_frame(scs_timestamp_t time_us, float speed) {
    scs_telemetry_frame_start_t start{};
    start.simulation_time = time_us;
    fire(SCS_TELEMETRY_EVENT_frame_start, &start);
    channel(SCS_TELEMETRY_TRUCK_CHANNEL_speed, float_value(speed));
    channel(SCS_TELEMETRY_TRUCK_CHANNEL_wheel_steering, float_value(0.05f), 0);
    scs_value_t placement{};
    placement.type = SCS_VALUE_TYPE_dplacement;
    placement.value_dplacement.position = {1000.5, 20.0, -3000.25};
    placement.value_dplacement.orientation = {0.5f, 0.0f, 0.0f};
    channel(SCS_TELEMETRY_TRUCK_CHANNEL_world_placement, placement);
    fire(SCS_TELEMETRY_EVENT_frame_end, nullptr);
}

// ---------------------------------------------------------------- pipe reader

const wchar_t kPipe[] = L"\\\\.\\pipe\\RoadPilot.Telemetry.v1";

HANDLE create_server() {
    return CreateNamedPipeW(kPipe, PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 0, 65536, 0, nullptr);
}

bool read_exact(HANDLE pipe, void* buffer, DWORD size) {
    auto* bytes = static_cast<unsigned char*>(buffer);
    DWORD done = 0;
    while (done < size) {
        DWORD read = 0;
        if (!ReadFile(pipe, bytes + done, size - done, &read, nullptr) || read == 0) return false;
        done += read;
    }
    return true;
}

// Reads frames until `count` were received, `stop_at` matched, or the connection ends.
std::vector<std::string> read_frames(HANDLE pipe, size_t count, const char* stop_at = nullptr) {
    std::vector<std::string> frames;
    while (frames.size() < count) {
        unsigned char prefix[4];
        if (!read_exact(pipe, prefix, 4)) break;
        const DWORD length = prefix[0] | (prefix[1] << 8) | (prefix[2] << 16) | (DWORD(prefix[3]) << 24);
        std::string payload(length, '\0');
        if (!read_exact(pipe, payload.data(), length)) break;
        frames.push_back(payload);
        if (stop_at && payload.find(stop_at) != std::string::npos) break;
    }
    return frames;
}

unsigned long long number_after(const std::string& json, const char* key) {
    const std::string pattern = std::string("\"") + key + "\":";
    const size_t at = json.find(pattern);
    return at == std::string::npos ? 0 : std::stoull(json.substr(at + pattern.size()));
}

bool contains(const std::string& haystack, const char* needle) {
    return haystack.find(needle) != std::string::npos;
}

// Plays frames on the "game thread" (this thread) while another thread reads them.
std::vector<std::string> exchange(HANDLE server, size_t count, scs_timestamp_t& time_us, float speed,
                                  const char* stop_at = nullptr) {
    std::vector<std::string> frames;
    std::thread reader([&] { frames = read_frames(server, count, stop_at); });
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
    while (std::chrono::steady_clock::now() < deadline) {
        play_frame(time_us += 16667, speed);
        std::this_thread::sleep_for(std::chrono::milliseconds(17));
        // Stop playing once the reader has everything (it exits the thread).
        if (WaitForSingleObject(reader.native_handle(), 0) == WAIT_OBJECT_0) break;
    }
    reader.join();
    return frames;
}

// Accepts the plugin's connection while the game keeps playing frames; the plugin only
// connects after it has a frame to send.
bool accept_while_playing(HANDLE server, scs_timestamp_t& time_us) {
    bool connected = false;
    std::thread accept([&] {
        connected = ConnectNamedPipe(server, nullptr) || GetLastError() == ERROR_PIPE_CONNECTED;
    });
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
    while (WaitForSingleObject(accept.native_handle(), 0) != WAIT_OBJECT_0 &&
           std::chrono::steady_clock::now() < deadline) {
        play_frame(time_us += 16667, 21.0f);
        std::this_thread::sleep_for(std::chrono::milliseconds(17));
    }
    if (WaitForSingleObject(accept.native_handle(), 0) != WAIT_OBJECT_0) {
        CancelSynchronousIo(accept.native_handle());
    }
    accept.join();
    return connected;
}

}  // namespace

int main() {
    std::setvbuf(stdout, nullptr, _IONBF, 0);
    HMODULE dll = LoadLibraryA(ROADPILOT_PLUGIN_PATH);
    if (!dll) {
        std::printf("FAILED: cannot load %s\n", ROADPILOT_PLUGIN_PATH);
        return 1;
    }
    typedef SCSAPI_RESULT_FPTR(init_t)(scs_u32_t, const scs_telemetry_init_params_t*);
    typedef SCSAPI_VOID_FPTR(shutdown_t)(void);
    auto init = reinterpret_cast<init_t>(GetProcAddress(dll, "scs_telemetry_init"));
    auto shutdown = reinterpret_cast<shutdown_t>(GetProcAddress(dll, "scs_telemetry_shutdown"));
    check(init && shutdown, "exports present");

    scs_telemetry_init_params_v100_t params;
    params.common.game_name = "Euro Truck Simulator 2";
    params.common.game_id = SCS_GAME_ID_EUT2;
    params.common.game_version = SCS_MAKE_VERSION(1, 21);
    params.common.log = log_line;
    params.register_for_event = register_event;
    params.unregister_from_event = unregister_event;
    params.register_for_channel = register_channel;
    params.unregister_from_channel = unregister_channel;

    // Unknown games are rejected.
    scs_telemetry_init_params_v100_t other = params;
    other.common.game_id = "some_other_game";
    check(init(SCS_TELEMETRY_VERSION_1_01, &other) == SCS_RESULT_unsupported, "unknown game rejected");
    check(init(SCS_MAKE_VERSION(2, 0), &params) == SCS_RESULT_unsupported, "unknown API version rejected");

    HANDLE server = create_server();
    if (server == INVALID_HANDLE_VALUE) {
        std::printf("FAILED: cannot create the RoadPilot pipe (is RoadPilot or the reader running?)\n");
        return 1;
    }

    check(init(SCS_TELEMETRY_VERSION_1_01, &params) == SCS_RESULT_ok, "init succeeds");
    check(g_events.count(SCS_TELEMETRY_EVENT_frame_end) == 1, "frame end registered");

    configuration(SCS_TELEMETRY_CONFIG_truck,
                  {named(SCS_TELEMETRY_CONFIG_ATTRIBUTE_brand,
                         [] { scs_value_t v{}; v.type = SCS_VALUE_TYPE_string; v.value_string.value = "Scania"; return v; }()),
                   named(SCS_TELEMETRY_CONFIG_ATTRIBUTE_fuel_capacity, float_value(800.0f)),
                   named(SCS_TELEMETRY_CONFIG_ATTRIBUTE_wheel_steerable,
                         [] { scs_value_t v{}; v.type = SCS_VALUE_TYPE_bool; v.value_bool.value = 1; return v; }(), 0)});
    scs_value_t income{};
    income.type = SCS_VALUE_TYPE_u64;
    income.value_u64.value = 24360;
    scs_value_t city{};
    city.type = SCS_VALUE_TYPE_string;
    city.value_string.value = "Hamburg";
    configuration(SCS_TELEMETRY_CONFIG_job, {named(SCS_TELEMETRY_CONFIG_ATTRIBUTE_source_city, city),
                                             named(SCS_TELEMETRY_CONFIG_ATTRIBUTE_income, income)});
    fire(SCS_TELEMETRY_EVENT_started, nullptr);

    std::printf("-- step 1\n");
    // 1. Many frames over one persistent connection.
    scs_timestamp_t time_us = 0;
    check(accept_while_playing(server, time_us), "plugin connects");
    std::vector<std::string> first = exchange(server, 20, time_us, 20.0f);
    check(first.size() == 20, "20 frames on one connection");
    if (first.size() == 20) {
        const std::string& f = first.back();
        check(contains(f, "\"schemaVersion\":2"), "schema 2");
        check(contains(f, "\"game\":\"ets2\"") && contains(f, "\"gameVersion\":\"1.21\""), "game identity");
        check(contains(f, "\"pluginState\":\"running\""), "running");
        check(contains(f, "\"speedMps\":20"), "speed channel");
        check(contains(f, "\"wheelSteeringRot\":0.05"), "steering of the steerable wheel");
        check(contains(f, "\"placement\":{\"x\":1000.5,\"y\":20,\"z\":-3000.25,\"heading\":0.5"), "placement");
        check(contains(f, "\"brand\":\"Scania\"") && contains(f, "\"fuelCapacityL\":800"), "truck config");
        check(contains(f, "\"job\":{\"state\":\"active\"") && contains(f, "\"sourceCity\":\"Hamburg\"") &&
                  contains(f, "\"income\":24360"),
              "job config");
        check(!contains(f, "adblueL"), "unsupported channel stays absent");
        check(number_after(f, "sequence") > number_after(first.front(), "sequence"), "sequence increases");
        check(number_after(f, "session") == number_after(first.front(), "session"), "one session");
        check(number_after(f, "simulationTimeUs") > 0, "simulation timestamp");
    }
    const unsigned long long session = first.empty() ? 0 : number_after(first.back(), "session");
    const unsigned long long last_sequence = first.empty() ? 0 : number_after(first.back(), "sequence");

    std::printf("-- step 2\n");
    // 2. The reader restarts: the plugin reconnects and continues the same session.
    DisconnectNamedPipe(server);
    CloseHandle(server);
    server = create_server();
    check(server != INVALID_HANDLE_VALUE, "reader restarts");
    check(accept_while_playing(server, time_us), "plugin reconnects");
    std::vector<std::string> second = exchange(server, 5, time_us, 22.0f);
    check(second.size() == 5, "frames after reconnect");
    if (!second.empty()) {
        check(number_after(second.front(), "session") == session, "same session after reconnect");
        check(number_after(second.front(), "sequence") > last_sequence, "sequence continues after reconnect");
    }

    std::printf("-- step 3\n");
    // 3. Pause, then a delivered job.
    fire(SCS_TELEMETRY_EVENT_paused, nullptr);
    scs_value_t revenue{};
    revenue.type = SCS_VALUE_TYPE_s64;
    revenue.value_s64.value = 25000;
    std::vector<scs_named_value_t> attributes{named(SCS_TELEMETRY_GAMEPLAY_EVENT_ATTRIBUTE_revenue, revenue),
                                              scs_named_value_t{}};
    scs_telemetry_gameplay_event_t delivered{SCS_TELEMETRY_GAMEPLAY_EVENT_job_delivered, attributes.data()};
    fire(SCS_TELEMETRY_EVENT_gameplay, &delivered);
    configuration(SCS_TELEMETRY_CONFIG_job, {});  // the game clears the job after delivery
    // Frames queued in the pipe before the pause are read first; stop at the paused one.
    std::vector<std::string> third = exchange(server, 1000, time_us, 0.0f, "\"pluginState\":\"paused\"");
    check(!third.empty(), "frames while paused");
    if (!third.empty()) {
        const std::string& f = third.back();
        check(contains(f, "\"pluginState\":\"paused\""), "paused state");
        check(contains(f, "\"state\":\"delivered\"") && contains(f, "\"deliveredRevenue\":25000"),
              "delivered job stays visible");
    }

    std::printf("-- step 4\n");
    // 4. Shutdown stops the worker and closes the pipe.
    shutdown();
    std::vector<std::string> after = read_frames(server, 1);
    check(after.empty(), "pipe closed after shutdown");
    CloseHandle(server);

    std::printf("-- step 5\n");
    // 5. The plugin can be initialized again (SDK reinit cycle).
    g_events.clear();
    g_channels.clear();
    check(init(SCS_TELEMETRY_VERSION_1_00, &params) == SCS_RESULT_ok, "re-init succeeds");
    shutdown();
    FreeLibrary(dll);

    if (g_failures == 0) std::printf("All plugin host tests passed.\n");
    return g_failures == 0 ? 0 : 1;
}
