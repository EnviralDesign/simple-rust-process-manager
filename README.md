# Simple Rust Process Manager

A lightweight, cross-platform process manager built with Rust and Dioxus. Perfect for managing multiple development services, Docker containers, and background processes from a single GUI.

![Process Manager](https://img.shields.io/badge/rust-1.70+-orange.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- **Process Management**: Start, stop, and restart multiple processes with a single click
- **Docker Integration**: Seamlessly control Docker containers alongside regular processes
- **Live Log Streaming**: View real-time output from any managed process
- **Status Monitoring**: Visual indicators show running/stopped state for each process
- **Portable Configuration**: JSON config file lives next to the executable for easy portability
- **Global Controls**: Start All, Stop All, Restart All buttons for quick environment setup
- **Graceful Shutdown**: Regular processes are killed on app close; Docker containers persist

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/simple-rust-process-manager.git
cd simple-rust-process-manager

# Build release version
cargo build --release

# The executable will be at target/release/simple-rust-process-manager.exe (Windows)
# or target/release/simple-rust-process-manager (Linux/macOS)
```

### Pre-built Binaries

Check the [Releases](https://github.com/yourusername/simple-rust-process-manager/releases) page for pre-built binaries.

## Usage

1. **First Run**: Launch the executable. If no `processes.json` exists next to it, one will be created automatically.

2. **Adding Processes**: Click the "+" button to add a new process entry. Fill in:
   - **Name**: A friendly name for the process
   - **Command**: The command to run (e.g., `npm run dev`)
   - **Working Directory**: Where to run the command from
   - **Type**: Either "Process" or "Docker"

3. **Managing Processes**:
   - Click on a process in the left sidebar to view its logs
   - Use the play/stop/restart buttons on each process card
   - Use global controls in the header for batch operations

4. **Docker Containers**: For Docker entries, specify the container name. The manager will use `docker start/stop/restart` commands.

## Configuration

The `processes.json` file structure:

```json
{
  "processes": [
    {
      "id": "uuid-here",
      "name": "Frontend Dev Server",
      "command": "npm run dev",
      "working_directory": "C:/projects/my-app/frontend",
      "process_type": "Process",
      "auto_start": false
    },
    {
      "id": "uuid-here",
      "name": "PostgreSQL",
      "command": "my-postgres-container",
      "working_directory": "",
      "process_type": "Docker",
      "auto_start": false
    }
  ]
}
```

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

- **Dioxus Desktop**: Native GUI framework for Rust
- **Tokio**: Async runtime for process management
- **Serde**: JSON serialization for config persistence
- **portable-pty**: Cross-platform PTY support for log streaming

## License

MIT License - feel free to use this for any purpose.

## Contributing

Contributions welcome! Please open an issue or PR.
