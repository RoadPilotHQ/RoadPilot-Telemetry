# RoadPilot telemetry protocol — schema 2

Schema 2 is the frame format written by the current plugin. It replaces the three-value schema 1 draft (see [PROTOCOL-DRAFT.md](PROTOCOL-DRAFT.md)) while keeping the same transport.

The definition lives in three places that are tested against each other:

- `client/src/v2.rs`: Rust model, decoding and validation (the reference for consumers).
- `plugin/src/frame_v2.{hpp,cpp}`: C++ model and serializer used by the plugin.
- `schemas/fixtures/*.json`: shared frames. The C++ tests require the serializer to produce them byte for byte (after removing whitespace); the Rust tests decode them and check every value.

`schemas/telemetry-frame-v2.schema.json` is a JSON Schema for documentation and external tooling.

## Transport

| Item | Value |
|---|---|
| Endpoint | Windows named pipe `\\.\pipe\RoadPilot.Telemetry.v1` (the `.v1` names the transport, which is unchanged) |
| Roles | Desktop reader = server (one instance, local clients only); plugin = client |
| Framing | 4-byte unsigned little-endian payload length, then that many bytes of UTF-8 JSON |
| Frame size | 1 byte to 64 KiB; a started frame must complete within 1 s |
| Connection | **Persistent.** The plugin opens the pipe once and writes every frame on that connection. |
| Reconnect | If no reader is available, the plugin retries every 1 s. If a write fails, it closes the handle and reconnects after 250 ms. |
| Rate | One frame per game physics frame, at most ~60 per second. Unsent frames are replaced by newer ones; the plugin never queues. |
| Malformed data | The reader closes that connection and waits for the next one. |

## Compatibility

- Every frame carries `protocol: "roadpilot.telemetry"` and `schemaVersion`.
- The reader accepts schema **1** and **2**. Schema 1 frames are upgraded to the schema 2 model with only speed, RPM and gear present.
- Unknown schema versions are rejected (fail closed). Unknown fields are ignored, so new optional fields can be added within schema 2 without breaking older readers.
- A field changing meaning or unit requires a new schema version.

## Ordering and timestamps

| Field | Type | Meaning |
|---|---|---|
| `session` | integer ≤ 2^53−1 | Random id per plugin initialization. Mandatory in schema 2. |
| `sequence` | integer ≤ 2^53−1 | Strictly increasing within a session, also across reconnects. |
| `simulationTimeUs` | integer ≤ 2^53−1 | SDK simulation time of the frame (µs). Can restart from zero; use `sequence` for ordering. |
| `gameTimeMin` | integer | In-game minutes since 00:00 of the first in-game day. |

The reader accepts a frame if it starts a new session or has a higher sequence than the last accepted frame of the same session; older or repeated frames are dropped without closing the connection. A session that was replaced by another one is retired and never accepted again (the last 8 are remembered), so a late frame of a finished game cannot pass as a new session. The reader adds its own monotonic receive time; it is never sent over the wire.

Schema 1 frames have no session. For them, ordering is checked within one connection only (the v1 plugin opened one connection per frame).

## Transport clock (QPC)

Optional since the plugin release after `febef76`, still schema 2. Windows `QueryPerformanceCounter` (QPC) is one system-wide monotonic counter, so values taken in the game process and in the reader's process are directly comparable on the same machine. The client verifies this in `client/tests/qpc_cross_process.rs`.

| Field | Type | Meaning |
|---|---|---|
| `producedQpc` | integer 1..2^53−1 | QPC ticks when the game finished this frame (`FRAME_END`, game thread). |
| `sentQpc` | integer 1..2^53−1 | QPC ticks immediately before the plugin's worker serialized and wrote the frame. |
| `qpcFrequency` | integer 1..2^53−1 | QPC ticks per second (`QueryPerformanceFrequency`), fixed at boot. |

The reader adds `received_qpc` together with its monotonic receive time, after reading and decoding, when it hands the frame on (never sent over the wire). From the three stamps it derives, with `client::qpc::evaluate`:

- **plugin processing** = `sentQpc − producedQpc`: waiting for the worker, its 60 Hz pacing and a blocked previous write;
- **transport** = `received − sentQpc`: serialization, the write, the pipe, the reader's read and decode;
- **age at receive** = `received − producedQpc`: the frame's absolute age; `qpc::age_at` gives it for any later QPC time.

Ticks convert exactly as `ticks / frequency` seconds in integer arithmetic (whole seconds plus the remainder in nanoseconds, truncated); it cannot overflow. A timing is valid only if all stamps are present, `qpcFrequency` equals the reader's own frequency, `producedQpc ≤ sentQpc ≤ received`, and the produced-to-received span is at most 60 s. Anything else is an error, never a duration: a negative, backwards or implausible difference is never reported as a low latency.

The plugin omits a stamp it cannot take or that exceeds 2^53−1, and the frequency if it is zero. The reader removes a value of 0 or above 2^53−1 as invalid (`dropped_fields`) and keeps the frame; a negative, fractional or non-integer value makes the frame malformed.

| Plugin | Reader | Result |
|---|---|---|
| previous (no clock fields) | current | Frames decode unchanged; timing is `Missing(Produced)`. |
| current | current | Frames decode with the stamps; timing is evaluated. |
| current | previous (`ca64c8e`) | Unknown fields are ignored; frames decode unchanged (checked: the previous client passes its tests against the current full fixture). |
| any | without QPC (non-Windows) | `received_qpc` is absent; timing is `Missing(Received)`. |

A frame without a valid timing remains usable for display and dry-run diagnostics. Its age is unknown, so a consumer that makes control decisions must not treat it as fresh on that basis; RoadPilot will require a valid timing before any live output may be enabled.

## Validation

- **Whole frame rejected:** wrong `protocol`, unsupported `schemaVersion`, missing `session` in schema 2, integers above 2^53−1, malformed JSON or wrong JSON types.
- **Single value removed:** non-finite numbers and values outside the plausibility bounds below. The frame is kept and the removed paths are reported (`dropped_fields`). The bounds are corruption guards, not game limits.
- Inputs that overshoot their range by at most 0.001 are clamped; further out they are removed.
- Strings longer than 256 bytes are removed.

## Field reference

All fields except the envelope are optional. **Absent means the game did not provide the value** (or it was removed as invalid); it never means zero. Sections without any value are omitted. Values are raw SDK quantities.

### Envelope

| Field | Type | Notes |
|---|---|---|
| `protocol` | string | `roadpilot.telemetry` |
| `schemaVersion` | integer | `2` |
| `session`, `sequence` | integer | See ordering |
| `game` | `ets2` \| `ats` | |
| `gameVersion` | string | SDK telemetry game version `major.minor` (e.g. ETS2 `1.21`), not the game patch number |
| `pluginState` | `running` \| `paused` | From the SDK started/paused events |
| `simulationTimeUs`, `gameTimeMin` | integer | See ordering |
| `producedQpc`, `sentQpc`, `qpcFrequency` | integer | See transport clock |
| `localScale` | number | Map scale applied to time and distance |

### `truck`

| Field | Unit | Bounds | SDK channel |
|---|---|---|---|
| `speedMps` | m/s, negative = reverse | ±150 | `truck.speed` |
| `engineRpm` | rpm | 0…20 000 | `truck.engine.rpm` |
| `gear` | >0 forward, 0 neutral, <0 reverse | −32…64 | `truck.engine.gear` |
| `displayedGear` | as above | −32…64 | `truck.displayed.gear` |
| `engineEnabled`, `electricEnabled` | bool | | `truck.engine.enabled`, `truck.electric.enabled` |
| `fuelL` | litres | 0…10 000 | `truck.fuel.amount` |
| `fuelRangeKm` | km (game estimate) | 0…100 000 | `truck.fuel.range` |
| `fuelConsumptionLPerKm` | l/km (game average) | 0…10 | `truck.fuel.consumption.average` |
| `fuelWarning` | bool | | `truck.fuel.warning` |
| `adblueL` | litres | 0…2 000 | `truck.adblue` (not in ATS) |
| `oilTemperatureC`, `waterTemperatureC` | °C | −100…500 | `truck.oil.temperature`, `truck.water.temperature` |
| `oilPressurePsi` | psi | 0…1 000 | `truck.oil.pressure` |
| `batteryVoltageV` | V | 0…100 | `truck.battery.voltage` |
| `brakeAirPressurePsi` | psi | 0…1 000 | `truck.brake.air.pressure` |
| `brakeTemperatureC` | °C | −100…2 000 | `truck.brake.temperature` |
| `parkingBrake`, `motorBrake` | bool | | `truck.brake.parking`, `truck.brake.motor` |
| `retarderLevel` | step, 0 = off | 0…64 | `truck.brake.retarder` |
| `cruiseControlMps` | m/s, **0 = off** | 0…150 | `truck.cruise_control` |
| `odometerKm` | km | 0…10^8 | `truck.odometer` |

The SDK states that pressures, temperatures and voltages are only loosely approximated, and that ATS fuel consumption is mostly static.

### `inputs`

| Field | Range | SDK channel |
|---|---|---|
| `steeringInput` | −1…1, **counterclockwise: positive = left** | `truck.input.steering` |
| `throttleInput`, `brakeInput`, `clutchInput` | 0…1 | `truck.input.*` |
| `steeringEffective` | −1…1, positive = left | `truck.effective.steering` |
| `throttleEffective`, `brakeEffective`, `clutchEffective` | 0…1 | `truck.effective.*` |

`*Input` is what the player gives; `*Effective` is what the simulation uses (interpolation, cruise control, automatic clutch). The effective brake excludes retarder, parking and engine brake. These are **normalized controls, not angles**.

### `motion`

| Field | Unit | Bounds | SDK channel |
|---|---|---|---|
| `localVelocityMps` | {x,y,z} m/s, vehicle space | each ±200 | `truck.local.velocity.linear` |
| `localAccelerationMps2` | {x,y,z} m/s², vehicle space | each ±1 000 | `truck.local.acceleration.linear` |
| `localAngularVelocityRps` | {x,y,z} rotations/s | each ±100 | `truck.local.velocity.angular` |
| `wheelSteeringRot` | rotations, −0.25…0.25, positive = left | | `truck.wheel.steering` of the first wheel marked steerable in the truck configuration |

`wheelSteeringRot` is the **physical road-wheel angle**, unlike `steeringInput`.

### `placement`

| Field | Unit |
|---|---|
| `x`, `y`, `z` | metres, world space (double precision) |
| `heading` | 0…1 rotation, counterclockwise from north |
| `pitch` | −0.25…0.25 rotation, positive = nose up |
| `roll` | −0.5…0.5 rotation, counterclockwise around the roll axis |

From `truck.world.placement`. The object is present only with all six values; one invalid value removes the whole placement.

### `truckConfig`

From the SDK `truck` configuration event: `brand`, `name` (strings), `fuelCapacityL`, `adblueCapacityL`, `rpmLimit` (numbers), `forwardGears`, `reverseGears`, `retarderSteps` (integers). An empty configuration (truck removed) clears all values.

Wheel geometry (additive, optional, raw SDK vehicle space: X right, Y up, **Z backwards**, metres; the plugin does no conversion):

| Field | Type / unit | Bounds | SDK configuration attribute |
|---|---|---|---|
| `wheelCount` | integer | 0…4096 | `wheels.count` |
| `hookPositionM` | {x,y,z} m | each ±50 | `hook.position` (trailer hook) |
| `wheels[]` | array indexed by SDK wheel index, at most 14 entries | | |
| `wheels[].positionM` | {x,y,z} m, wheel centre | each ±50 | `wheel.position` |
| `wheels[].radiusM` | m | 0.05…3 | `wheel.radius` |
| `wheels[].steerable` | bool | | `wheel.steerable` |
| `wheels[].powered` | bool | | `wheel.powered` |

`wheels` is sent only when the game provided `wheelCount`; its length is `min(wheelCount, 14)`. Entries beyond 14 are not reported, so `wheelCount` can be larger than `wheels.length`. An individual wheel field the game did not provide is absent (never zero). A list with more than 14 entries is removed as a whole; invalid single values are removed individually. Derived quantities such as the wheelbase are not part of the protocol; consumers calculate them.

### `job`

| Field | Type / unit | Source |
|---|---|---|
| `state` | `none` \| `active` \| `delivered` \| `cancelled` | Job configuration and gameplay events |
| `cargo` | string | config `cargo` |
| `cargoMassKg` | kg | config `cargo.mass` |
| `cargoDamage` | 0…1 | channel `job.cargo.damage` (only while `active`) |
| `cargoLoaded` | bool | config `cargo.loaded` |
| `sourceCity`, `sourceCompany`, `destinationCity`, `destinationCompany` | string, localized by the game | config |
| `income` | integer, game currency (ETS2 EUR, ATS USD) | config `income` |
| `deliveryTimeMin` | absolute in-game minute when the delivery window ends | config `delivery.time` |
| `plannedDistanceKm` | km | config `planned_distance.km` |
| `jobMarket` | string, e.g. `freight_market` | config `job.market` |
| `deliveredRevenue` | integer, game currency | `job.delivered` event `revenue` |
| `cancelPenalty` | integer, game currency | `job.cancelled` event `cancel.penalty` |

The `job` object is omitted until the game sent its first job configuration. `delivered` and `cancelled` stay until the next job starts, so the result of the last job remains visible.

### `navigation`

Route Advisor values as provided by the game: `distanceM` (metres), `timeS` (seconds), `speedLimitMps` (m/s; respects the in-game "Route Advisor speed limit" option). The SDK does not document what a speed limit of 0 means; it is passed through unchanged. Consumers that interpret 0 as "no limit shown" should treat that as unconfirmed until verified in the game.

## Coordinate conventions

From the SDK headers (`scssdk_value.h`):

- **Vehicle (local) space:** X right, Y up, **Z backwards**. Forward velocity is therefore `−localVelocityMps.z`.
- **World space:** X east, Y up, **Z south**. North is −Z.
- **Rotations** are stored in units of full turns, not degrees or radians. Heading is counterclockwise seen from above: 0 = north, 0.25 = west, 0.5 = south, 0.75 = east.

The protocol carries these raw values unchanged; any conversion is done and documented by the consumer.

## Not available from the SDK

The SCS Telemetry SDK 1.16 provides no channel for: weather, rain, day/night state, surrounding traffic, road or lane information, the navigation destination name, or the planned route geometry. Schema 2 therefore has no fields for them.
