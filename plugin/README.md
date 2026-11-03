# Native telemetry plugin

Windows SCS Telemetry SDK plugin for ETS2 and ATS. It registers speed, engine RPM, selected gear and lifecycle callbacks, then sends length-prefixed RoadPilot JSON frames to `\\.\pipe\RoadPilot.Telemetry.v1` from a dedicated worker thread.

## Build

Use Visual Studio C++ tools and CMake. Point `SCS_SDK_ROOT` at an extracted SCS SDK directory; SDK headers are intentionally not vendored. For example:

```powershell
cmake -S plugin -B build/plugin -A x64 -DSCS_SDK_ROOT="C:/path/to/scs_sdk_1_16"
cmake --build build/plugin --config Release
```

The DLL builds successfully and has passed a live smoke test in American Truck Simulator 1.62.1.1s with the console reader. Broader ETS2/ATS version validation remains. The build does not install the DLL or change game directories.

The build output is named `roadpilot-telemetry.dll`.
