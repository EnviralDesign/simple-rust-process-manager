# Simple Rust Process Manager

A fast, no-nonsense process manager built with Rust + egui/eframe. Start whole stacks in one click, watch logs live, and actually stop everything cleanly (even messy toolchains like npm/uv). It's small, snappy, and built for real dev workflows.

![Process Manager](https://img.shields.io/badge/rust-1.70+-orange.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

![Process Manager UI](images/image1.png)

## Features

- **Process Management**: Start, stop, and restart multiple processes with a single click
- **Entry Editing**: Update process definitions in place (no delete/recreate cycle)
- **Real Process Tree Control (Windows)**: Uses Job Objects to kill entire trees so nothing gets orphaned
- **Docker Integration**: Seamlessly control Docker containers alongside regular processes
- **Live Log Streaming**: View real-time output from any managed process
- **Status Monitoring**: Visual indicators show running/stopped state for each process
- **Error Attention**: Flashes the taskbar icon on new errors when the app is unfocused
- **Managed Restart (Per Entry)**: Opt-in restart when a process/container goes down unexpectedly
- **Portable Configuration**: JSON config file lives next to the executable for easy portability
- **Local REST Control API**: Optional loopback-only control surface for stack-wide and per-process actions
- **Agent Bootstrap Copy**: Copy a ready-to-paste AI agent skill block with the current host, port, endpoints, and process ids
- **Global Controls**: Start All, Stop All, Restart All buttons for quick environment setup
- **Graceful Shutdown**: Regular processes are killed on app close; Docker containers persist

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/EnviralDesign/simple-rust-process-manager.git
cd simple-rust-process-manager

# Build release version
cargo build --release

# The executable will be at target/release/simple-rust-process-manager.exe (Windows)
# or target/release/simple-rust-process-manager (Linux/macOS)
```

### Pre-built Binaries

Check the [Releases](https://github.com/EnviralDesign/simple-rust-process-manager/releases) page for pre-built binaries.

## Usage

1. **First Run**: Launch the executable. If no `processes.json` exists next to it, one will be created automatically.

2. **Adding Processes**: Click the "+" button to add a new process entry. Fill in:
   - **Name**: A friendly name for the process
   - **Command**: The command to run (e.g., `npm run dev`, `uv run dev`)
   - **Working Directory**: Where to run the command from
   - **Type**: Either "Process" or "Docker"
   - **Managed Restart**: Automatically restart this entry if it goes down

3. **Managing Processes**:
   - Click on a process in the left sidebar to view its logs
   - Use the play/stop/restart/edit/delete buttons on each process card
   - Use global controls in the header for batch operations

4. **Local API**:
   - Use the `Enable API` / `Disable API` control in the header to turn the localhost REST server on or off
   - Click `API Settings` to edit the port; the host is always fixed to `127.0.0.1`
   - Click `Copy Agent Skill` to copy a bootstrap payload for an AI engineer or agent

5. **Docker Containers**: For Docker entries, specify the container name. The manager will use `docker start/stop/restart` commands.

## Command Notes (Direct Spawn)

Commands are executed directly (no shell), which keeps process IDs tight and makes stop/kill reliable.  
That also means shell operators like `&&`, `|`, `>` aren't supported in the command box. If you need them, wrap the logic in a `.cmd`/`.bat`/`.ps1` or a script and call that script instead.

## Configuration

The `processes.json` file structure:

```json
{
  "stack_name": "My Stack",
  "remote_control": {
    "enabled": false,
    "port": 47821
  },
  "processes": [
    {
      "id": "uuid-here",
      "name": "Frontend Dev Server",
      "command": "npm run dev",
      "working_directory": "C:/projects/my-app/frontend",
      "process_type": "Process",
      "auto_start": false,
      "auto_restart": true
    },
    {
      "id": "uuid-here",
      "name": "PostgreSQL",
      "command": "my-postgres-container",
      "working_directory": "",
      "process_type": "Docker",
      "auto_start": false,
      "auto_restart": false
    }
  ]
}
```

Existing `processes.json` files from older versions are migrated automatically on startup. The new `remote_control` object is additive, so replacing the EXE does not require recreating your stack definition.

## Local REST API

When enabled, the manager starts a loopback-only HTTP server on `127.0.0.1:{port}`.

Read endpoints:

- `GET /health`
- `GET /processes`
- `GET /processes/{id}`
- `GET /processes/{id}/logs?limit=N`
- `GET /topology`

Control endpoints:

- `POST /stack/start`
- `POST /stack/stop`
- `POST /stack/restart`
- `POST /processes/{id}/start`
- `POST /processes/{id}/stop`
- `POST /processes/{id}/restart`

Notes:

- The API binds only to `127.0.0.1`
- Use stable process `id` values, not display names, for per-process actions
- Use `GET /processes/{id}/logs?limit=N` to read the last N log lines for one component; default is 200 and max is 1000
- Control calls are fire-and-poll: after a `POST`, poll `GET /processes` or `GET /health`
- `Copy Agent Skill` copies a text block that includes the current host, port, endpoint topology, and known process ids

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+N` | Add new process |
| `Ctrl+S` | Start all processes |
| `Ctrl+X` | Stop all processes |
| `Ctrl+R` | Restart all processes |

## Development

```bash
# Run in development mode with hot reload
cargo run

# Run tests
cargo test

# Build release
cargo build --release
```

## Architecture

- **egui / eframe**: Native immediate-mode desktop GUI for Rust
- **Tokio**: Async runtime for process management
- **Serde**: JSON serialization for config persistence
- **portable-pty**: Cross-platform PTY support for log streaming

## License

MIT License - feel free to use this for any purpose.

## Donations & Support

If this saves you time or pain, you can support the work here:

- Patreon (recurring)
- GitHub Sponsors (recurring)
- PayPal (one-time)

Links:
- https://www.patreon.com/EnviralDesign
- https://github.com/sponsors/EnviralDesign
- https://www.paypal.com/donate?hosted_button_id=RP8EJAHSDTZ86

## Contributing

Contributions welcome! Please open an issue or PR.
