# Native telemetry plugin

Windows SCS Telemetry SDK plugin for ETS2 and ATS. It registers the truck, input, motion, placement, navigation, game-time and cargo-damage channels plus the configuration (truck including wheel geometry, job) and gameplay (job delivered/cancelled) events, and writes RoadPilot [schema 2](../docs/PROTOCOL-V2.md) frames to `\.\pipe\RoadPilot.Telemetry.v1` over one persistent connection from a worker thread. See [`../docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md) for threading and failure handling.

## Files

- `src/plugin.cpp` — SDK glue: registration, callbacks, transport worker.
- `src/frame_v2.hpp`, `src/frame_v2.cpp` — data model and JSON serializer, no SDK dependency.
- `tests/frame_v2_tests.cpp` — serializer tests against `../schemas/fixtures`.
- `tests/plugin_host_tests.cpp` — loads the built DLL in a fake SCS host and talks to it over the real pipe. With `--drive <seconds>` it only plays frames into the plugin, for a reader in another process (the client's cross-process clock test).

## Build and test

Use Visual Studio C++ tools and CMake. Point `SCS_SDK_ROOT` at an extracted [SCS SDK 1.16](https://download.eurotrucksimulator2.com/scs_sdk_1_16.zip); SDK headers are intentionally not stored in this repository.

```powershell
cmake -S plugin -B plugin/build -A x64 -DSCS_SDK_ROOT="C:/path/to/scs_sdk_1_16"
cmake --build plugin/build --config Release
ctest --test-dir plugin/build -C Release --output-on-failure
```

`-DROADPILOT_BUILD_PLUGIN=OFF` builds and runs only the serializer tests (no SDK needed). Close RoadPilot and the console reader before running the host test; it uses the real pipe name.

The build output is `plugin/build/Release/roadpilot-telemetry.dll`. The build does not install the DLL or change game directories.

## Verification status

The schema 2 plugin passes the automated tests above. It has **not** been run in ETS2 or ATS yet, so the wheel geometry is unverified in a game as well. The earlier schema 1 plugin (speed, RPM, gear) was live-tested in ATS 1.62.x.
