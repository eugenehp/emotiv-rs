# Changelog

## 0.0.3 — 2026-03-19

### Bugfixes
- **Prevent re-auth warnings from killing active sessions**: The Cortex service can send `ACCESS_RIGHT_GRANTED` (code 9), `HEADSET_CONNECTED` (code 104), and `HEADSET_SCANNING_FINISHED` (code 142) warnings at any time — including while a session is active. Previously each handler unconditionally triggered `authorize()`, `query_headsets()`, or `refresh_headset_list()`, which could invalidate the current cortex token or create a duplicate session. Now each handler checks the current state first and skips the action if a token or session is already established.

## 0.0.1 — 2026-03-17

Initial release.

### Features
- Full Emotiv Cortex API client over WebSocket (JSON-RPC 2.0)
- Authentication flow: hasAccessRight → requestAccess → authorize
- Headset management: query, connect, disconnect, refresh
- Session lifecycle: create, close
- Data streaming: EEG, motion, device info, performance metrics, band power, mental commands, facial expressions, system events
- Subscribe / unsubscribe to any combination of streams
- Recording: create, stop, export (CSV / EDF)
- Markers: inject, update
- Profile management: query, create, load, unload, save
- BCI training: mental command and facial expression training via sys stream
- Advanced BCI: active actions, sensitivity get/set, brain map, training threshold
- Record management: query records, request download
- Headset clock sync
- Signal simulator for all data types (no hardware needed)
- CLI binary (`emotiv-cli`) with `--simulate` mode
- TUI binary (`emotiv-tui`) with EEG waveforms, metrics bars, band power view
- 8 example scripts mirroring the official cortex-example Python examples
- 69 unit tests, 17 integration tests, 2 doc-tests
