# Claude Code Instructions for Miscord

This file contains important instructions and guidelines for Claude Code when working on this project.

## Process Management

**CRITICAL: Never kill processes by pattern matching names.**

- Do NOT use `pkill -f <pattern>` or similar commands that match process names
- This can accidentally kill Docker containers, editors, and other unrelated processes
- Always get the specific PID first, then kill only that exact PID

### Safe process management:

```bash
# 1. Find the specific PID
ps aux | grep "target/debug/miscord-server" | grep -v grep

# 2. Kill only the specific PID (replace 12345 with actual PID)
kill 12345
```

### Unsafe (DO NOT USE):

```bash
# These are DANGEROUS - they can kill unrelated processes
pkill -f miscord          # NO!
pkill -9 -f miscord       # NO!
killall miscord           # NO!
```

## Development Setup

### Database
- The project uses PostgreSQL via Docker
- Start with: `colima start && docker compose up -d postgres`
- Database URL: `postgres://miscord:miscord@localhost:5434/miscord`

### Running the Server
```bash
cargo run -p miscord-server
```

### Running the Client
```bash
cargo run -p miscord-client
```

## Architecture Notes

- **miscord-server**: Backend API server (Rust/Axum)
- **miscord-client**: Desktop client (Rust/egui)
- **miscord-protocol**: Shared types
- **miscord-media**: Media handling (audio/video codecs)

## Video Capture

The project uses GStreamer for video capture (not nokhwa) for proper hardware acceleration and WebRTC support.
