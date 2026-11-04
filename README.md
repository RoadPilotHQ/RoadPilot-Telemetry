# RoadPilot-Telemetry

RoadPilot-Telemetry is the standalone telemetry component of the RoadPilot ecosystem. It contains two independent parts:

- a native ETS2/ATS game plugin that reads channels through the official SCS Telemetry SDK;
- a desktop-side Rust reader that receives, validates and orders RoadPilot telemetry frames.

The telemetry is **read-only**: nothing in this repository sends input to the game.

## Status

| Area | Implemented | Verified |
|---|---|---|
| Protocol schema 2 (truck, inputs, motion, placement, truck config, job, navigation, game time) | Yes, see [PROTOCOL-V2.md](docs/PROTOCOL-V2.md) | Automated tests only |
| Persistent pipe connection with reconnect | Yes | Automated tests (real pipe, fake SCS host) |
| Schema 1 compatibility in the reader | Yes | Automated tests |
| Plugin in a running game | — | **Schema 1 plugin:** live-tested in ATS 1.62.x (speed, RPM, gear). **Schema 2 plugin: not yet tested in ETS2 or ATS.** |

The schema 2 plugin and reader pass automated tests without a game: the serializer produces the shared fixture frames byte for byte, the reader decodes and validates them, and a fake SCS host loads the built DLL and exchanges frames with it over the real pipe, including reconnects, pause, job events and re-initialization. No field of schema 2 has been checked against a running game yet; see [the manual test](docs/ARCHITECTURE.md#manual-game-test).

## Layout

- `plugin/` — native in-game SCS plugin (C++17, CMake), its serializer and tests.
- `client/` — Rust crate `roadpilot-telemetry-client`: frame model, decoding, validation, ordering, named-pipe server and a console reader.
- `schemas/` — JSON Schema files and the shared test fixtures.
- `docs/` — [architecture](docs/ARCHITECTURE.md), [protocol schema 2](docs/PROTOCOL-V2.md), [schema 1 draft](docs/PROTOCOL-DRAFT.md).

## Build and test

Rust client (any Windows machine with Rust):

```powershell
cargo fmt --manifest-path client/Cargo.toml --check
cargo clippy --manifest-path client/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path client/Cargo.toml
cargo build --release --manifest-path client/Cargo.toml   # console reader
```

Plugin and C++ tests (Visual Studio C++ tools, CMake, an extracted [SCS SDK](https://download.eurotrucksimulator2.com/scs_sdk_1_16.zip)):

```powershell
cmake -S plugin -B plugin/build -A x64 -DSCS_SDK_ROOT="C:/path/to/scs_sdk_1_16"
cmake --build plugin/build --config Release
ctest --test-dir plugin/build -C Release --output-on-failure
```

The serializer tests alone need no SDK: configure with `-DROADPILOT_BUILD_PLUGIN=OFF`. The plugin host test uses the real pipe name, so RoadPilot and the console reader must be closed while it runs.

## Installation

Copy `plugin/build/Release/roadpilot-telemetry.dll` into the game's `bin/win_x64/plugins/` folder (create it if needed), separately for ETS2 and ATS. Nothing in this repository installs or modifies game files. The game asks once to confirm the use of SDK plugins.

## Compatibility

- One plugin binary serves ETS2 and ATS; it reports the game and the SDK telemetry game version in every frame.
- The plugin accepts SDK telemetry API 1.00 and 1.01. Channels a game does not provide (for example AdBlue in ATS) are simply absent.
- The reader accepts schema 1 frames from the first plugin and schema 2 frames.

## Licensing

- Project licence: undecided.
- SCS SDK: © SCS Software, MIT-style licence included in the SDK archive (`sdk_license.txt`). The SDK is not stored in this repository; the plugin is built against a locally extracted copy.
- Rust dependencies report MIT, Apache-2.0, Unlicense or Apache-2.0 with LLVM exception terms (see `client/README.md`).

Review distribution terms before publishing built binaries.
