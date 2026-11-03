# RoadPilot-Telemetry

RoadPilot-Telemetry will be the standalone telemetry component for the RoadPilot ecosystem. It will contain two independent parts:

- a native ETS2/ATS game plugin that reads channels through the SCS telemetry API;
- a desktop-side reader that receives and validates RoadPilot telemetry frames.

## Status

Implementation draft. The desktop client contains a Rust decoder, Windows named-pipe server and a small console reader. The native plugin registers core SCS telemetry channels and publishes draft frames from a worker thread. Both components build successfully and have passed a live smoke test in American Truck Simulator 1.62.1.1s. The plugin was manually placed in the ATS plugin folder for that test; the project does not install or update game files automatically.

The official [SCS Telemetry SDK documentation](https://modding.scssoft.com/wiki/Documentation/Engine/SDK/Telemetry) lists SDK 1.16 as stable as of 3 November 2026. This is an investigation baseline, not a confirmed game-version support promise. See [the architecture notes](docs/ARCHITECTURE.md) and [protocol draft](docs/PROTOCOL-DRAFT.md).

## Layout

- `plugin/` — native in-game SCS plugin.
- `client/` — desktop-side transport reader and normalized telemetry API.
- `schemas/` — versioned wire schemas after the protocol is approved.
- `docs/` — architecture, protocol decisions, SDK and lifecycle notes.

Build the Windows console reader with `cargo build --release --manifest-path client/Cargo.toml`. Direct and transitive crate metadata lists MIT/Apache-2.0, MIT, Unlicense, or Apache-2.0 with LLVM exception terms; the project license remains undecided.

## Decisions still open

- supported ETS2/ATS game-version range and installation approach;
- frame-rate, freshness threshold, and exact channel set/availability semantics;
- project license and distribution terms.

Do not copy or redistribute SDK files or build artifacts until the applicable notices and project distribution requirements have been reviewed. The SCS SDK archive includes its own license notice.
