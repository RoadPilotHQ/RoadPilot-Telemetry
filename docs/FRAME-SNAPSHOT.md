# Frame snapshot in the console reader

The console reader (`client/src/main.rs`, binary `roadpilot-telemetry-client`) can print the newest received frame once, as **one JSON line** that can be copied into a measurement sheet. It exists for manual in-game measurements (for example the road template validation of RoadPilot-Map). It does not change the protocol, writes no files and stores nothing.

## Use

1. Start the reader: `cargo run --release --manifest-path client/Cargo.toml`. Only one reader can own the pipe, so close RoadPilot first.
2. Start the game; the status line shows `LIVE`.
3. Press **Enter** in the reader's console window. The reader prints one line and keeps running. Each Enter prints one line.

Waiting for Enter happens on a separate thread; receiving frames and the status line continue without pause.

## Example

Synthetic values, not a recording:

```json
{"kind":"roadpilot.telemetry.snapshot","snapshotVersion":1,"status":"fresh","ageMs":12,"missing":[],"droppedFields":[],"frame":{"protocol":"roadpilot.telemetry","schemaVersion":2,"session":42,"sequence":7,"game":"ats","gameVersion":"1.5","pluginState":"running","simulationTimeUs":1000,"localScale":19.5,"placement":{"x":-12345.678901234567,"y":12.25,"z":98765.4321,"heading":0.1,"pitch":-0.0021,"roll":0.00037},"speedMps":0.0125,"localVelocityMps":{"x":0.001,"y":0.0,"z":-0.0125}}}
```

## Fields

| Field | Meaning |
|---|---|
| `kind`, `snapshotVersion` | `roadpilot.telemetry.snapshot`, layout version 1 |
| `status` | `fresh`: a new frame received within the last 2 s. `repeated`: the same frame (session and sequence) as the previous snapshot; no newer frame arrived. `stale`: the newest frame is older than 2 s (game paused or closed, plugin not sending). `no_frame`: nothing received yet; there is no `frame` object |
| `ageMs` | Milliseconds since this reader received the frame |
| `missing` | Fields a measurement needs that this frame does not contain: `session`, `gameVersion`, `placement`, `truck.speedMps`. Never filled with estimates |
| `droppedFields` | Fields the decoder removed as invalid (for example a `placement` with a value out of range; the whole placement is then removed) |
| `frame.session`, `frame.sequence` | Plugin session id and frame sequence number: the frame reference |
| `frame.game`, `frame.gameVersion` | `ets2`/`ats` and the SDK telemetry game version `major.minor` (**not** the game patch shown in the game) |
| `frame.pluginState`, `frame.simulationTimeUs`, `frame.localScale` | As in schema 2 |
| `frame.placement` | `x`, `y`, `z` in metres, world space (X east, Y up, Z south); `heading` 0…1 turn counterclockwise from north, `pitch`, `roll` in turns. From `truck.world.placement` |
| `frame.speedMps` | `truck.speedMps`, the speedometer speed in m/s (negative while reversing) |
| `frame.localVelocityMps` | `motion.localVelocityMps`, vehicle-space velocity in m/s; shows whether the truck really stands still |

Field names and units are those of schema 2 ([PROTOCOL-V2.md](PROTOCOL-V2.md)); `speedMps` and `localVelocityMps` are taken out of their `truck` and `motion` objects. Fields that the frame does not contain are left out, never written as 0. Job, navigation and truck configuration are not printed.

## Precision

The values are copied from the decoded frame **unchanged**:

- The plugin receives the position as `double` and the orientation as `float` (SCS SDK `dplacement`), and writes every number with `std::to_chars`: the shortest text that reads back to exactly the same value, with a decimal point, independent of the locale.
- The reader parses positions into `f64` and the orientation, speed and scale into `f32`, the same types the plugin sent.
- The snapshot writes them with `serde_json`: shortest round-trip text, decimal point, no thousands separator, independent of the system locale. `f32` values are written as `f32`, so no digits are added by widening to `f64`.

So nothing is rounded between the SDK and the snapshot. The data source limits the precision only by its own types: `float` for heading, pitch, roll and speed, `double` for the position. The status line of the reader still rounds `x`/`z` to whole metres for display only.

## Limits

- **The reference point of `placement` is not known.** The SDK channel gives the position of the truck, but schema 2 does not say which point of the truck that is, and it does not carry the wheel, cabin or hook positions of the truck configuration.
- **A common coordinate basis with the map is not established.** `placement` uses the SDK's world space and metres. That this is the same space and unit as the map data read by RoadPilot-Map is an assumption until it is shown by measurement.
- A snapshot is one frame. It says nothing about lanes, markings or the road: the SDK provides no road or lane information.
- A precise snapshot is therefore **not** evidence of a lane width, an axis position or any lane geometry. It must not be used for active steering or a safety-critical lane departure warning.
- `fresh` means only that the reader got the frame recently; check `speedMps` and `localVelocityMps` yourself to see that the truck stands still.
- Copy snapshots into your own notes only as needed. They are not written to a file by the reader; do not commit them.
