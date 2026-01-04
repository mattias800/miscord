# Miscord

A self-hosted Discord clone built in Rust with voice, video, and screen sharing support.

## Features

- **User Accounts** - Registration, authentication, user profiles
- **Text Channels** - Real-time messaging with reactions and replies
- **Direct Messages** - Private conversations between users
- **Servers** - Create communities with multiple channels
- **Voice Channels** - Real-time voice chat with WebRTC
- **Video Calls** - Webcam support in voice channels and DMs
- **Screen Sharing** - Share your screen or specific windows
- **Capture Card Support** - Use external capture devices (Elgato 4K series, etc.)

## Architecture

The project is organized as a Rust workspace with multiple crates:

```
miscord/
├── crates/
│   ├── miscord-server/     # Backend server (API, WebSocket, signaling)
│   ├── miscord-client/     # Desktop client (GUI, media capture)
│   ├── miscord-protocol/   # Shared types and protocol definitions
│   └── miscord-media/      # Audio/video encoding and processing
```

## Requirements

### Server
- Rust 1.75+
- PostgreSQL 14+

### Client
- Rust 1.75+
- Platform-specific media libraries:
  - **Linux**: ALSA, PulseAudio, V4L2
  - **macOS**: Core Audio, AVFoundation
  - **Windows**: Windows Media Foundation, DirectShow

## Getting Started

### Server Setup

1. Set up PostgreSQL and create a database:
   ```bash
   createdb miscord
   ```

2. Configure environment variables:
   ```bash
   export DATABASE_URL="postgres://localhost/miscord"
   export JWT_SECRET="your-secret-key-here"
   export BIND_ADDRESS="0.0.0.0:8080"
   ```

3. Run the server:
   ```bash
   cargo run -p miscord-server
   ```

### Client Setup

1. Run the client:
   ```bash
   cargo run -p miscord
   ```

2. Connect to your server at `http://localhost:8080` (or your server URL)

## Development

### Building

```bash
# Build all crates
cargo build

# Build in release mode
cargo build --release
```

### Testing

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p miscord-protocol
```

### Database Migrations

Migrations are run automatically when the server starts. To create new migrations:

```bash
cd crates/miscord-server
sqlx migrate add <migration_name>
```

## Configuration

### Server Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `sqlite:miscord.db` |
| `JWT_SECRET` | Secret key for JWT tokens | Random (insecure) |
| `BIND_ADDRESS` | Server listen address | `0.0.0.0:8080` |
| `STUN_SERVERS` | Comma-separated STUN server URLs | Google STUN |
| `TURN_SERVERS` | TURN server configuration | None |

### TURN Server Setup (Optional)

For voice/video to work reliably behind NAT, you may need a TURN server:

```bash
# Using coturn
export TURN_SERVERS="turn:your-turn-server.com:3478"
export TURN_USERNAME="username"
export TURN_CREDENTIAL="password"
```

## Technology Stack

- **Backend**: Axum, SQLx, WebRTC-rs
- **Frontend**: egui (native GUI)
- **Database**: PostgreSQL
- **Real-time**: WebSockets, WebRTC
- **Audio**: Opus codec via opus-rs
- **Media Capture**: nokhwa, xcap, cpal

## License

MIT License - see [LICENSE](LICENSE) for details.
