# emotiv

Async Rust client, CLI, and TUI for streaming EEG and BCI data from
[Emotiv](https://www.emotiv.com/) headsets via the
[Cortex API](https://emotiv.gitbook.io/cortex-api/) WebSocket.

## Supported Hardware

| Model | EEG Channels | BCI | Notes |
|---|---|---|---|
| EPOC X | 14 | ✓ | Full EEG, motion, performance metrics, mental commands |
| EPOC+ | 14 | ✓ | Same protocol as EPOC X |
| Insight | 5 | ✓ | Lightweight 5-channel headset |
| EPOC Flex | 32 | ✓ | Research-grade flexible cap |

## Features

- **Full Cortex API** — authentication, headset management, sessions, data streaming, records, markers, profiles, BCI training
- **All data streams** — EEG, motion, device info, performance metrics, band power, mental commands, facial expressions
- **Signal simulator** *(feature = `simulate`)* — synthetic EEG/motion/metrics/band power/mental command data for testing without hardware
- **TUI** — real-time terminal UI with scrolling EEG waveforms, metrics bars, and band power display
- **CLI** — headless streaming with formatted output
- **Example scripts** — mirror the official [cortex-example](https://github.com/Emotiv/cortex-example) Python examples

## Setting Up Cortex API Credentials

All communication with a real Emotiv headset goes through the **Emotiv Cortex
service**, a local WebSocket server (`wss://localhost:6868`) that ships with
the EMOTIV Launcher. To use it you need a **Client ID** and **Client Secret**:

### Step 1 — Install EMOTIV Launcher

Download from [emotiv.com/emotiv-launcher](https://www.emotiv.com/products/emotiv-launcher),
install, and **log in** with your EmotivID. Accept the EULA / Privacy Policy
when prompted.

### Step 2 — Create a Cortex App

1. Go to <https://www.emotiv.com/my-account/cortex-apps/> (log in with your EmotivID).
2. Click **"Create a new Cortex App"**.
3. Fill in an app name and description, then click **Create**.
4. Copy the **Client ID** and **Client Secret** shown on the confirmation page.

> **Keep your Client Secret private.** Do not commit it to version control.

### Step 3 — Set environment variables

```bash
export EMOTIV_CLIENT_ID="your_client_id_here"
export EMOTIV_CLIENT_SECRET="your_client_secret_here"
```

Or pass them directly in code via [`CortexClientConfig`](src/client.rs).

### Step 4 — Approve access (first run only)

The first time your app connects, the Cortex service will show an approval
dialog in the EMOTIV Launcher. Click **Approve**. This only needs to be done
once per Client ID.

### Optional — Virtual headset (no hardware)

If you don't have a physical headset, you can create a virtual BrainWear®
device inside the Launcher:
[instructions](https://emotiv.gitbook.io/emotiv-launcher/devices-setting-up-virtual-brainwear-r/creating-a-virtual-brainwear-device).

## Quick Start

### Simulation mode (no hardware or API keys needed)

Requires the `simulate` cargo feature:

```bash
# CLI — prints simulated data to stdout
cargo run --bin emotiv-cli --features simulate -- --simulate

# TUI — interactive terminal dashboard with simulated signals
cargo run --bin emotiv-tui --features simulate -- --simulate

# Standalone example
cargo run --example simulate --features simulate
```

### Real headset

Make sure the EMOTIV Launcher is running, your headset is connected, and the
environment variables from Step 3 above are set:

```bash
# CLI
cargo run --bin emotiv-cli

# TUI
cargo run --bin emotiv-tui

# Examples
cargo run --example sub_data
cargo run --example client_mode
cargo run --example client_mode -- --resilient
cargo run --example record
cargo run --example marker
cargo run --example mental_command_train
cargo run --example live_advance
cargo run --example query_records
cargo run --example facial_expression_train
```

## Library Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
emotiv = "0.0.1"

# Include the signal simulator for offline testing:
# emotiv = { version = "0.0.1", features = ["simulate"] }
```

### Connecting to a headset

```rust
use emotiv::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Credentials from https://www.emotiv.com/my-account/cortex-apps/
    let config = CortexClientConfig {
        client_id: std::env::var("EMOTIV_CLIENT_ID")?,
        client_secret: std::env::var("EMOTIV_CLIENT_SECRET")?,
        ..Default::default()
    };

    let client = CortexClient::new(config);
    let (mut rx, handle) = client.connect().await?;

    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::SessionCreated(_) => {
                handle.subscribe(&["eeg", "mot", "met", "pow"]).await?;
            }
            CortexEvent::Eeg(data) => {
                println!("EEG: {:?}", &data.samples[..5.min(data.samples.len())]);
            }
            CortexEvent::Disconnected => break,
            _ => {}
        }
    }
    Ok(())
}
```

### Resilient client (auto-reconnect + health checks)

```rust
use emotiv::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Loads from cortex.toml or EMOTIV_CLIENT_ID / EMOTIV_CLIENT_SECRET
    let config = CortexConfig::discover(None)?;

    let (client, mut events) = ResilientClient::connect(config).await?;

    let mut conn_events = client.connection_event_receiver();
    tokio::spawn(async move {
        while let Ok(event) = conn_events.recv().await {
            println!("Connection event: {:?}", event);
        }
    });

    while let Ok(event) = events.recv().await {
        match event {
            CortexEvent::SessionCreated(_) => {
                // Re-subscribe after each reconnect (session changes)
                client.subscribe(&["eeg", "met"]).await?;
            }
            CortexEvent::Disconnected => break,
            _ => {}
        }
    }

    Ok(())
}
```

### Using the simulator (feature = `simulate`)

```rust
use emotiv::simulator::{SimulatorConfig, spawn_simulator};
use emotiv::types::CortexEvent;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::channel(256);
    spawn_simulator(SimulatorConfig::default(), tx);

    while let Some(event) = rx.recv().await {
        if let CortexEvent::Eeg(data) = event {
            println!("EEG: {:?}", &data.samples[..5]);
        }
    }
}
```

## TUI Keybindings

### All modes

| Key | Action |
|-----|--------|
| `Tab` | Cycle views (EEG → Metrics → BandPower [→ SimControl]) |
| `+`/`=` | Zoom out (increase µV scale) |
| `-` | Zoom in (decrease µV scale) |
| `a` | Auto-scale Y axis to current peak |
| `v` | Toggle smooth overlay |
| `p` | Pause streaming |
| `r` | Resume streaming |
| `c` | Clear waveform buffers |
| `q`/`Esc` | Quit |

### Simulate mode — brain states (`--features simulate`)

| Key | State | Description |
|-----|-------|-------------|
| `1` | Relaxed | Strong alpha, low beta. Eyes-closed calm. |
| `2` | Focused | Strong beta, low alpha. Concentrated task. |
| `3` | Excited | Strong beta + gamma. High arousal. |
| `4` | Drowsy | Strong theta, low beta. Falling asleep. |
| `5` | Meditative | Strong alpha + theta. Deep relaxation. |

Brain state changes smoothly interpolate all signals — EEG waveform shape,
band power distribution, and performance metrics all update in real time.

### Simulate mode — artifacts & events

| Key | Action |
|-----|--------|
| `b` | Eye blink artifact (200 ms frontal spike on AF3/AF4) |
| `j` | Jaw clench artifact (500 ms EMG burst on temporal T7/T8) |
| `m` | Cycle mental command (neutral → push → pull → lift → drop) |
| `f` | Cycle facial expression (neutral → smile → surprise → frown → clench) |
| `n` / `N` | Decrease / increase noise level |
| `g` / `G` | Decrease / increase signal gain |

## Project Structure

```
src/
├── lib.rs          # Library root with prelude
├── client.rs       # WebSocket client, auth flow, event dispatch
├── protocol.rs     # JSON-RPC request builders, Cortex API constants
├── simulator.rs    # Signal simulator (feature = "simulate")
├── types.rs        # All data types and events
├── main.rs         # CLI binary (emotiv-cli)
└── bin/
    └── tui.rs      # TUI binary (emotiv-tui)

examples/
├── sub_data.rs             # Subscribe to EEG, motion, metrics, band power
├── record.rs               # Create/stop recordings, export to CSV/EDF
├── marker.rs               # Inject time markers into recordings
├── mental_command_train.rs # Train mental command BCI actions
├── facial_expression_train.rs # Train facial expression actions
├── live_advance.rs         # Live mental command with sensitivity
├── query_records.rs        # Query and download saved records
└── simulate.rs             # Standalone simulator demo (feature = "simulate")
```

## Examples ↔ cortex-example Mapping

| Rust Example | Python Original | Description |
|---|---|---|
| `sub_data` | `sub_data.py` | Subscribe to EEG, motion, metrics, band power |
| `record` | `record.py` | Create/stop recordings, export to CSV/EDF |
| `marker` | `marker.py` | Inject time markers into recordings |
| `mental_command_train` | `mental_command_train.py` | Train mental command BCI actions |
| `live_advance` | `live_advance.py` | Live mental command with sensitivity control |
| `query_records` | `query_records.py` | Query and download saved records |
| `facial_expression_train` | `facial_expression_train.py` | Train facial expression actions |
| `simulate` | *(new)* | Standalone signal simulator demo |

## Testing

```bash
cargo test                       # without simulator (75 tests)
cargo test --features simulate   # with simulator (87 tests)
```

With `--features simulate`, the full suite covers:

- **Protocol builders** (31 tests) — every JSON-RPC request builder, constants, CSV vs EDF export
- **Stream data parsing** (8 tests) — EEG, motion, dev, metrics, band power, mental command, facial expression, sys
- **Message handler** (20 tests) — auth flow, headset query, session creation, subscribe/unsubscribe, profile CRUD, record CRUD, marker injection, query records, MC active actions/sensitivity, warnings, errors
- **Simulator** (8 unit + 4 integration) — signal range/variance/length, async event production, 5-channel mode, disabled streams, timestamp monotonicity
- **Serde round-trips** (9 tests) — Record, Marker, HeadsetInfo, EegData, MentalCommandData, FacialExpressionData deserialization from real Cortex JSON
- **Doc-tests** (1) — quick-start example compile-check

## Citation

If you use this software in academic work, please cite it:

```bibtex
@software{hauptmann2026emotiv,
  author       = {Hauptmann, Eugene},
  title        = {emotiv-rs: Async Rust Client for Emotiv EEG Headsets via the Cortex API},
  year         = {2026},
  url          = {https://github.com/eugenehp/emotiv-rs},
  version      = {0.0.1},
  license      = {MIT}
}
```

## License

MIT
