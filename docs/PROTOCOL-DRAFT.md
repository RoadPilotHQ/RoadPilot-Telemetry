# RoadPilot telemetry protocol — draft

This document records a candidate JSON frame contract. It is still **draft** and must be reviewed before treating v1 as a stable wire format. The desktop reader currently decodes this candidate; the plugin does not emit it yet.

## Stream framing draft

The current Windows reader expects a byte stream on `\\.\pipe\RoadPilot.Telemetry.v1`. Each frame is a 4-byte unsigned little-endian payload length followed by exactly that many UTF-8 JSON bytes. Lengths of zero or greater than 64 KiB are rejected. A partial header or payload must complete within one second once the frame has started; an incomplete frame disconnects the client. The desktop reader hosts the pipe, accepts one client, rejects remote clients, and requires the first pipe instance. The native plugin connects and publishes frames from a worker thread, never from an SCS callback.

## Proposed frame semantics

- Identify the frame with protocol ID `roadpilot.telemetry` and schema version `1`.
- Include game identity (`ets2` or `ats`), plugin lifecycle state, and a monotonically increasing per-session sequence.
- Encode sequence as a JSON integer no greater than JavaScript's exact integer limit; the Rust model uses `u64` and rejects larger values.
- The initial domain model uses speed in meters per second (including negative reverse speed), engine RPM, and selected engine gear. These types and meanings are present in the [official SCS SDK 1.16 archive](https://download.eurotrucksimulator2.com/scs_sdk_1_16.zip), in its common truck channel definitions; confirm them against each supported game version before claiming compatibility.
- Preserve unsupported or unavailable fields as absent (`None` in Rust, omitted from JSON); do not coerce missing SCS values to zero.
- Keep the SDK callback timestamp, in-game time, and desktop receive time distinct. The desktop reader should determine freshness using its own monotonic clock unless source timestamp semantics are verified.
- Normalize SDK-specific data before exposing it to the RoadPilot app. A schema revision must not encode C++ struct layout or an external plugin's memory map.
- Define compatibility rules: breaking changes require a new major protocol version; additive optional fields may be introduced compatibly. Unknown major versions fail closed.

## Connection lifecycle

The reader should distinguish at least: not configured, plugin not detected, game stopped, connecting, connected, stale, disconnected, and error. A successful connection check must not be reported until it has actually occurred.

## Decisions required before stabilizing v1

1. Define the supported ETS2/ATS game-version range; the current live smoke test covers ATS 1.62.1.1s.
2. Approve the first field set and units from the selected SDK headers and documentation.
3. Set frame-rate and freshness limits.
4. Approve the schema and compatibility policy before treating the draft as stable.

The Rust client has a JSON decoder and a Windows named-pipe server. It validates protocol ID, schema revision, sequence range, finite speed, and non-negative RPM. The JSON Schema draft is in `../schemas/telemetry-frame-v1.schema.json`. The native plugin publisher is implemented as a draft in `../plugin/` and has passed an ATS 1.62.1.1s smoke test with the reader. The SCS sample shared-memory layout and RenCloud's mapping are references for investigation only; RoadPilot's current transport draft is the named pipe described above.
