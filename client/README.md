# Desktop telemetry reader

`roadpilot-telemetry-client` is the Rust side of the RoadPilot telemetry protocol:

- `TelemetryFrame` (schema 2 model, `src/v2.rs`) and the schema 1 upgrade (`src/v1.rs`);
- `decode_frame`: envelope checks (protocol, schema version, session, safe integers) and field validation that removes non-finite or implausible values and reports them as `dropped_fields`;
- `ordering::FrameOrder`: session + sequence ordering across reconnects;
- `named_pipe::serve_latest`: the Windows pipe server, publishing the newest in-order frame with its monotonic receive time through a Tokio `watch` channel;
- a console reader (`src/main.rs`) showing game, schema, speed, RPM, gear, fuel and position.

The pipe accepts one local client, rejects remote clients and requires the first pipe instance, so only one reader (RoadPilot or the console reader) can run at a time. See [`../docs/PROTOCOL-V2.md`](../docs/PROTOCOL-V2.md) for the wire format and [`../docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md) for behaviour and tests.

RoadPilot consumes this crate as a git dependency pinned to a commit.

The dependency set is locked in `Cargo.lock`. Its crate metadata reports MIT/Apache-2.0, MIT, Unlicense, or Apache-2.0 with LLVM exception terms, with Unicode-3.0 included in the `unicode-ident` license expression. The project license is still undecided.
