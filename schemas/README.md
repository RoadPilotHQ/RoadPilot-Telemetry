# Protocol schemas

- `telemetry-frame-v2.schema.json` — JSON Schema of the current frame (schema 2). The normative description with units and coordinate conventions is [`../docs/PROTOCOL-V2.md`](../docs/PROTOCOL-V2.md).
- `telemetry-frame-v1.schema.json` — schema 1, still accepted by the reader for compatibility.
- `fixtures/` — shared test frames. The C++ serializer tests must produce `frame-v2-full.json` and `frame-v2-minimal.json` exactly (ignoring whitespace); the Rust tests decode all fixtures. Change them only together with both implementations.

Keep SDK headers and ABI structs out of this directory.
