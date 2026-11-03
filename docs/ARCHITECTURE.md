# Architecture

## Components

### Native game plugin (`plugin/`)

The plugin is loaded by ETS2/ATS and negotiates a supported SCS telemetry API version. It adapts SCS callbacks into RoadPilot-owned telemetry values. It remains independent of the desktop UI and map-processing code.

SCS documents repeated API initialization/shutdown cycles and single-threaded callbacks that are not thread-safe. The plugin must handle lifecycle transitions repeatedly and keep blocking work out of game callbacks. This last point is a design constraint inferred from the callback/threading model.

### Desktop reader (`client/`)

The Rust crate hosts a Windows named-pipe server, decodes length-prefixed JSON frames, and publishes the latest validated frame through a Tokio watch channel. Its console reader displays live values and marks data stale after two seconds without a frame. It rejects remote clients, enforces one server instance, and disconnects malformed or incomplete frames. RoadPilot UI integration remains unimplemented. The UI must not consume raw SCS structs or transport-specific layouts.

The transport draft uses `\\.\pipe\RoadPilot.Telemetry.v1`: the desktop reader is the inbound server and the game plugin is the client. Each frame has a 4-byte little-endian length followed by JSON. The server grants authenticated local users read/write access and rejects remote clients; this supports game and reader processes with different Windows tokens while keeping the endpoint local. The server also requires the first pipe instance. These options are backed by [Tokio's Windows named-pipe API](https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/struct.ServerOptions.html) and the [Microsoft named-pipe API](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea). The native plugin keeps writes off SCS callbacks because pipe writes may block when buffers fill; it uses a dedicated worker thread.

### Protocol (`schemas/`)

The protocol belongs to RoadPilot, not to SCS or another plugin. A draft JSON schema and byte-stream framing rule are in `schemas/` and `docs/PROTOCOL-DRAFT.md`; neither is approved as a stable external contract. The named pipe carries the normalized JSON model, not SCS memory layouts.

## Boundaries

- Telemetry and map analysis remain independent. The map parser consumes installed game files; telemetry reports live game state.
- No plugin installer, updater, or modification of game directories is part of the initial scaffold.
- The desktop reader should report unavailable, stale, and disconnected data explicitly. Missing SDK channels must never be presented as real zeroes.
- Real ETS2/ATS compatibility can only be claimed after testing supported game versions with the matching plugin and reader.

## SCS SDK baseline

The [official SCS documentation](https://modding.scssoft.com/wiki/Documentation/Engine/SDK/Telemetry) lists Telemetry SDK 1.16 as stable and says its archive contains documentation and examples. The SDK's `telemetry_mem` sample describes shared memory as reducing output precision and adding latency. That example is not adopted as RoadPilot's wire format. See [the protocol draft](PROTOCOL-DRAFT.md).
## Native plugin (Windows)

The native plugin under `plugin/` registers the SCS common truck channels for speed, engine RPM and selected gear, along with started/paused/frame-end events. SCS callbacks only update a small in-memory snapshot; a dedicated worker connects to the desktop reader's named pipe and writes the framed JSON. SDK headers are supplied locally with `SCS_SDK_ROOT` at build time and are not stored in this repository.

The plugin has not been exercised inside either game. Installation, reconnect behavior against a real desktop reader and ETS2/ATS compatibility remain to be validated.
