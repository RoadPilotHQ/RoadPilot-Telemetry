# Desktop telemetry reader

`roadpilot-telemetry-client` holds the first candidate JSON frame model, decoder, and Windows named-pipe server. It validates protocol ID, schema revision, sequence range, finite speed, and non-negative engine RPM. Desktop receive time remains separate from the serialized frame.

The console reader hosts the local pipe and displays live ATS/ETS2 game, speed, RPM and gear values. The plugin-to-reader connection was smoke-tested in ATS 1.62.1.1s. The pipe grants read/write access to authenticated local users and rejects remote clients. RoadPilot UI integration and broader game-version validation remain open. Rust was chosen for this client crate so the RoadPilot Tauri shell can consume it later. See [`../docs/PROTOCOL-DRAFT.md`](../docs/PROTOCOL-DRAFT.md).

The current dependency set is locked in `Cargo.lock`. Its crate metadata reports MIT/Apache-2.0, MIT, Unlicense, or Apache-2.0 with LLVM exception terms, with Unicode-3.0 included in the `unicode-ident` license expression. The project license is still undecided.
