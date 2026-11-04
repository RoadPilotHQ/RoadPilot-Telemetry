# Architecture

## Components

### Native game plugin (`plugin/`)

The plugin is loaded by ETS2/ATS through the official SCS Telemetry SDK. It negotiates telemetry API 1.00/1.01, accepts only the game ids of ETS2 and ATS, and registers events and channels during `scs_telemetry_init`.

Threading follows the SDK rules (all callbacks on the game's main thread, API not thread-safe):

1. Channel, configuration and gameplay callbacks write only into a stage object owned by the game thread. No locking per channel.
2. On every `FRAME_END` the stage is copied once into a shared snapshot under a mutex and the sequence number advances. Per-frame values are plain data (no allocation); strings from configuration events are copied only when they changed.
3. A worker thread serializes the newest snapshot (schema 2 JSON) and writes it to the pipe. Pipe I/O never runs on the game thread, so a missing or slow reader cannot stall the game.

Failure handling:

- Only `FRAME_END`, `started` and `paused` are required. Missing channels, configuration or gameplay events (older game versions, ATS without AdBlue) leave the corresponding values absent; the number of missing channels is written to the game log once.
- The worker keeps one persistent connection. Without a reader it retries every second; after a failed write it closes the handle and reconnects after 250 ms. Transport changes are written to the game log once per change.
- On `scs_telemetry_shutdown` the worker is stopped (a blocked write is cancelled with `CancelSynchronousIo`) and all state is reset, so the SDK's repeated init/shutdown cycles are supported.
- During pause the SDK sends no channel data but still sends frame events; the plugin keeps sending frames with `pluginState: "paused"` and the last values.

The serializer (`frame_v2.{hpp,cpp}`) has no SDK dependency, uses `std::to_chars` (locale-independent) and omits non-finite numbers and empty sections.

### Desktop reader (`client/`)

The Rust crate hosts the named pipe (`named_pipe::serve_latest`), decodes and validates frames (`decode_frame`), and publishes only the newest in-order frame through a Tokio `watch` channel together with its monotonic receive time.

- One pipe instance, local clients only (`reject_remote_clients`), first instance required, authenticated local users may write (the game may run with a different token).
- A fresh pipe instance is created for each connection. With the first reader's approach of reusing a disconnected instance, the automated tests reproduced lost frames after a reconnect (the previous client's end-of-stream error was returned on the next client's first read).
- Ordering across reconnects uses `session` + `sequence` (`ordering::FrameOrder`).
- Validation rejects whole frames only for envelope errors and removes single invalid values otherwise (`TelemetryFrame::sanitize`).

### Protocol (`schemas/`, `docs/PROTOCOL-V2.md`)

The protocol belongs to RoadPilot, not to SCS or another plugin, and carries normalized JSON, never SDK memory layouts. The shared fixtures in `schemas/fixtures` are produced by the C++ serializer tests and decoded by the Rust tests, so the two implementations cannot drift apart unnoticed.

## Boundaries

- Telemetry and map analysis remain independent. RoadPilot-Map parses installed game files; telemetry reports live game state.
- No plugin installer, updater, or modification of game directories.
- No input injection or vehicle control.
- Real ETS2/ATS compatibility can only be claimed after testing supported game versions with the matching plugin and reader.

## Tests

| Test | What it covers | Needs |
|---|---|---|
| `cargo test` (client) | Decoding, validation, schema 1 upgrade, ordering, real-pipe persistent connection, reconnect, malformed frames, second server | Windows |
| `roadpilot_frame_tests` | Serializer output equals the shared fixtures; escaping; non-finite values; large integers; locale independence | C++ compiler |
| `roadpilot_plugin_host_tests` | Built DLL in a fake SCS host: init/version checks, channels, config and gameplay events, persistent pipe, reconnect after reader restart, pause, delivered job, shutdown, re-init | SCS SDK headers; RoadPilot and the console reader closed |

## Manual game test

Automated tests cannot prove that the game delivers each value. For each game (ETS2 and ATS):

1. Build the plugin, copy `roadpilot-telemetry.dll` into `bin/win_x64/plugins/`, close other telemetry readers.
2. Start the console reader (`client`), then the game; confirm the SDK plugin dialog. Expect "connected to the desktop reader" in the game log and `LIVE … v2` in the reader.
3. Load a truck and drive: speed, RPM, gear, fuel and position must change plausibly.
4. Accept a job: check source, destination, cargo, income in RoadPilot.
5. Pause (Esc) and resume; exit and restart the game with the reader open.
6. Record per game which fields were confirmed, with the game version.

## SCS SDK baseline

The [official SCS documentation](https://modding.scssoft.com/wiki/Documentation/Engine/SDK/Telemetry) lists Telemetry SDK 1.16 as stable. Channel names, units and coordinate conventions in this repository are taken from the 1.16 headers (`scssdk_telemetry_truck_common_channels.h`, `scssdk_telemetry_common_configs.h`, `scssdk_telemetry_common_gameplay_events.h`, `scssdk_value.h`). The SDK's `telemetry_mem` sample describes shared memory as reducing output precision and adding latency; it is not used.
