#include <windows.h>

#include <chrono>
#include <atomic>
#include <condition_variable>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>

#include "scssdk.h"
#include "scssdk_telemetry.h"
#include "common/scssdk_telemetry_truck_common_channels.h"
#include "eurotrucks2/scssdk_eut2.h"
#include "amtrucks/scssdk_ats.h"

namespace {
constexpr wchar_t kPipeName[] = L"\\\\.\\pipe\\RoadPilot.Telemetry.v1";
constexpr std::uint32_t kMaxSafeSequence = 9007199254740991ULL;

struct Snapshot {
    const char* game = nullptr;
    bool running = false;
    bool have_speed = false;
    bool have_rpm = false;
    bool have_gear = false;
    float speed = 0;
    float rpm = 0;
    std::int32_t gear = 0;
    std::uint64_t sequence = 0;
};

std::mutex g_mutex;
std::condition_variable g_changed;
Snapshot g_snapshot;
std::thread g_worker;
HANDLE g_worker_handle = nullptr;
bool g_stopping = false;
bool g_dirty = false;
scs_log_t g_log = nullptr;
std::atomic<int> g_transport_state{0}; // 0 starting, 1 reader unavailable, 2 frame sent, 3 write failed
std::atomic<DWORD> g_transport_error{ERROR_SUCCESS};
int g_reported_transport_state = -1;

void log_message(scs_log_type_t type, const char* message) {
    if (g_log) g_log(type, message);
}

void report_transport_status() {
    const int state = g_transport_state.load(std::memory_order_relaxed);
    if (state == 0 || state == g_reported_transport_state) return;
    g_reported_transport_state = state;
    char message[192];
    const DWORD error = g_transport_error.load(std::memory_order_relaxed);
    if (state == 1) {
        std::snprintf(message, sizeof(message), "RoadPilot: desktop reader pipe unavailable (Windows error %lu).", static_cast<unsigned long>(error));
    } else if (state == 2) {
        std::snprintf(message, sizeof(message), "RoadPilot: first telemetry frame sent to desktop reader successfully.");
    } else {
        std::snprintf(message, sizeof(message), "RoadPilot: telemetry pipe write failed (Windows error %lu).", static_cast<unsigned long>(error));
    }
    log_message(state == 2 ? SCS_LOG_TYPE_message : SCS_LOG_TYPE_warning, message);
}

void publish_wakeup() {
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_dirty = true;
    }
    g_changed.notify_one();
}

void channel_float(const scs_string_t, const scs_u32_t, const scs_value_t* value, const scs_context_t context) {
    const auto field = reinterpret_cast<intptr_t>(context);
    std::lock_guard<std::mutex> lock(g_mutex);
    if (field == 1) {
        g_snapshot.have_speed = value != nullptr;
        if (value) g_snapshot.speed = value->value_float.value;
    } else {
        g_snapshot.have_rpm = value != nullptr;
        if (value) g_snapshot.rpm = value->value_float.value;
    }
}

void channel_gear(const scs_string_t, const scs_u32_t, const scs_value_t* value, const scs_context_t) {
    std::lock_guard<std::mutex> lock(g_mutex);
    g_snapshot.have_gear = value != nullptr;
    if (value) g_snapshot.gear = value->value_s32.value;
}

void telemetry_event(const scs_event_t event, const void*, const scs_context_t) {
    if (event == SCS_TELEMETRY_EVENT_started || event == SCS_TELEMETRY_EVENT_paused) {
        {
            std::lock_guard<std::mutex> lock(g_mutex);
            g_snapshot.running = event == SCS_TELEMETRY_EVENT_started;
            if (g_snapshot.sequence < kMaxSafeSequence) ++g_snapshot.sequence;
        }
        publish_wakeup();
    } else if (event == SCS_TELEMETRY_EVENT_frame_end) {
        {
            std::lock_guard<std::mutex> lock(g_mutex);
            if (g_snapshot.sequence < kMaxSafeSequence) ++g_snapshot.sequence;
        }
        publish_wakeup();
        report_transport_status();
    }
}

std::string make_frame(const Snapshot& s) {
    char json[512];
    char values[192] = "";
    bool comma = false;
    auto append = [&](const char* fmt, auto value) {
        const auto used = std::strlen(values);
        std::snprintf(values + used, sizeof(values) - used, fmt, value);
        comma = true;
    };
    if (s.have_speed) append(comma ? ",\"speedMps\":%.9g" : "\"speedMps\":%.9g", s.speed);
    if (s.have_rpm) append(comma ? ",\"engineRpm\":%.9g" : "\"engineRpm\":%.9g", s.rpm);
    if (s.have_gear) append(comma ? ",\"gear\":%d" : "\"gear\":%d", s.gear);
    const int len = std::snprintf(json, sizeof(json),
        "{\"protocol\":\"roadpilot.telemetry\",\"schemaVersion\":1,\"sequence\":%llu,\"game\":\"%s\",\"pluginState\":\"%s\",\"values\":{%s}}",
        static_cast<unsigned long long>(s.sequence), s.game ? s.game : "ets2", s.running ? "running" : "paused", values);
    return (len > 0 && static_cast<size_t>(len) < sizeof(json)) ? std::string(json, len) : std::string();
}

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

void transport_worker() {
    for (;;) {
        Snapshot snapshot;
        {
            std::unique_lock<std::mutex> lock(g_mutex);
            g_changed.wait(lock, [] { return g_stopping || g_dirty; });
            if (g_stopping) return;
            g_dirty = false;
            snapshot = g_snapshot;
        }
        const std::string payload = make_frame(snapshot);
        if (payload.empty()) continue;
        HANDLE pipe = CreateFileW(kPipeName, GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
        if (pipe == INVALID_HANDLE_VALUE) {
            g_transport_error.store(GetLastError(), std::memory_order_relaxed);
            g_transport_state.store(1, std::memory_order_relaxed);
            std::unique_lock<std::mutex> lock(g_mutex);
            g_changed.wait_for(lock, std::chrono::milliseconds(500), [] { return g_stopping; });
            if (g_stopping) return;
            g_dirty = true;
            continue;
        }
        const std::uint32_t length = static_cast<std::uint32_t>(payload.size());
        const unsigned char header[4] = {static_cast<unsigned char>(length), static_cast<unsigned char>(length >> 8), static_cast<unsigned char>(length >> 16), static_cast<unsigned char>(length >> 24)};
        DWORD write_error = ERROR_SUCCESS;
        const bool ok = write_all(pipe, header, sizeof(header), &write_error) && write_all(pipe, payload.data(), length, &write_error);
        if (ok) {
            g_transport_error.store(ERROR_SUCCESS, std::memory_order_relaxed);
            g_transport_state.store(2, std::memory_order_relaxed);
        } else {
            g_transport_error.store(write_error, std::memory_order_relaxed);
            g_transport_state.store(3, std::memory_order_relaxed);
        }
        CloseHandle(pipe);
        if (!ok) {
            std::unique_lock<std::mutex> lock(g_mutex);
            g_changed.wait_for(lock, std::chrono::milliseconds(250), [] { return g_stopping; });
            if (g_stopping) return;
            g_dirty = true;
        }
    }
}

void start_worker() {
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_stopping = false;
        g_dirty = true;
    }
    g_worker = std::thread(transport_worker);
    g_worker_handle = static_cast<HANDLE>(g_worker.native_handle());
}

void stop_worker() {
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_stopping = true;
    }
    g_changed.notify_all();
    if (g_worker_handle) CancelSynchronousIo(g_worker_handle);
    if (g_worker.joinable()) g_worker.join();
    g_worker_handle = nullptr;
}

} // namespace

SCSAPI_RESULT scs_telemetry_init(const scs_u32_t version, const scs_telemetry_init_params_t* params) {
    if (version != SCS_TELEMETRY_VERSION_1_00 && version != SCS_TELEMETRY_VERSION_1_01) return SCS_RESULT_unsupported;
    const auto* p = static_cast<const scs_telemetry_init_params_v100_t*>(params);
    if (!p || !p->common.game_id) return SCS_RESULT_invalid_parameter;
    const char* game = nullptr;
    if (std::strcmp(p->common.game_id, SCS_GAME_ID_EUT2) == 0) game = "ets2";
    else if (std::strcmp(p->common.game_id, SCS_GAME_ID_ATS) == 0) game = "ats";
    else return SCS_RESULT_unsupported;

    g_log = p->common.log;
    g_transport_state.store(0, std::memory_order_relaxed);
    g_transport_error.store(ERROR_SUCCESS, std::memory_order_relaxed);
    g_reported_transport_state = -1;
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_snapshot = Snapshot{};
        g_snapshot.game = game;
    }
    start_worker();

    if (p->register_for_event(SCS_TELEMETRY_EVENT_frame_end, telemetry_event, nullptr) != SCS_RESULT_ok ||
        p->register_for_event(SCS_TELEMETRY_EVENT_started, telemetry_event, nullptr) != SCS_RESULT_ok ||
        p->register_for_event(SCS_TELEMETRY_EVENT_paused, telemetry_event, nullptr) != SCS_RESULT_ok) {
        stop_worker();
        return SCS_RESULT_generic_error;
    }
    const auto register_channel = p->register_for_channel;
    const auto no_value = SCS_TELEMETRY_CHANNEL_FLAG_no_value;
    const bool channels_ok =
        register_channel(SCS_TELEMETRY_TRUCK_CHANNEL_speed, SCS_U32_NIL, SCS_VALUE_TYPE_float, no_value, channel_float, reinterpret_cast<void*>(1)) == SCS_RESULT_ok &&
        register_channel(SCS_TELEMETRY_TRUCK_CHANNEL_engine_rpm, SCS_U32_NIL, SCS_VALUE_TYPE_float, no_value, channel_float, reinterpret_cast<void*>(2)) == SCS_RESULT_ok &&
        register_channel(SCS_TELEMETRY_TRUCK_CHANNEL_engine_gear, SCS_U32_NIL, SCS_VALUE_TYPE_s32, no_value, channel_gear, nullptr) == SCS_RESULT_ok;
    if (!channels_ok) {
        stop_worker();
        return SCS_RESULT_generic_error;
    }
    log_message(SCS_LOG_TYPE_message, "RoadPilot telemetry plugin initialized; waiting for desktop reader.");
    return SCS_RESULT_ok;
}

SCSAPI_VOID scs_telemetry_shutdown(void) {
    stop_worker();
    {
        std::lock_guard<std::mutex> lock(g_mutex);
        g_snapshot = Snapshot{};
    }
    g_log = nullptr;
}
