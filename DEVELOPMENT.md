# Miscord Development Guide

This guide explains how to set up and run Miscord for development and testing.

## Prerequisites

- Rust (latest stable)
- Docker and Docker Compose
- A terminal that supports ANSI colors (optional, for pretty output)

## Quick Start

### Option 1: One Command Startup (Easiest)

Start everything with a single command - server and two clients with auto-login:

```bash
./dev-start.sh
```

This will:
1. Start PostgreSQL in Docker
2. Build server and client
3. Start the server
4. Seed test data (alice, bob, charlie accounts)
5. Launch two clients (Alice and Bob) that auto-login

Press `Ctrl+C` to stop all processes.

### Option 2: Full Development Environment

Run everything with a single command:

```bash
./scripts/dev-full.sh
```

This will:
1. Start PostgreSQL in Docker
2. Build and start the server (with automatic migrations)
3. Create test users (alice, bob, charlie)
4. Wait for you to launch clients

Then, in separate terminals:

```bash
# Terminal 2 - Launch client as Alice
./scripts/client-alice.sh

# Terminal 3 - Launch client as Bob
./scripts/client-bob.sh
```

### Option 2: Manual Setup

#### Step 1: Start the Database

```bash
docker-compose up -d postgres
```

#### Step 2: Start the Server

```bash
./scripts/dev-server.sh
```

#### Step 3: Seed Test Data

In a new terminal (while the server is running):

```bash
./scripts/seed-dev-data.sh
```

#### Step 4: Launch Clients

```bash
# Terminal for Alice
./scripts/client-alice.sh

# Terminal for Bob
./scripts/client-bob.sh
```

## Test Accounts

All test accounts use the password: `password123`

| Username  | Display Name      | Email               |
|-----------|-------------------|---------------------|
| alice     | Alice Developer   | alice@example.com   |
| bob       | Bob Tester        | bob@example.com     |
| charlie   | Charlie Observer  | charlie@example.com |

## Environment Variables

### Server

| Variable       | Default                    | Description                |
|----------------|----------------------------|----------------------------|
| DATABASE_URL   | postgres://...             | PostgreSQL connection URL  |
| JWT_SECRET     | (dev default)              | Secret for JWT signing     |
| BIND_ADDRESS   | 0.0.0.0:8080               | Server bind address        |
| RUST_LOG       | miscord_server=debug       | Log level configuration    |

### Client

| Variable              | Default               | Description                      |
|-----------------------|-----------------------|----------------------------------|
| MISCORD_SERVER_URL    | http://localhost:8080 | Server URL to connect to         |
| MISCORD_WINDOW_TITLE  | Miscord               | Custom window title              |
| MISCORD_PROFILE       | default               | Session profile name (for multi-client testing) |
| MISCORD_AUTO_LOGIN_USER | (none)              | Username for auto-login          |
| MISCORD_AUTO_LOGIN_PASS | (none)              | Password for auto-login          |

## Running Multiple Clients Manually

When testing features like video or screen sharing, you need multiple clients with different user sessions.
Use the `MISCORD_PROFILE` environment variable to keep sessions separate:

```bash
# Terminal 1 - Start the server
cargo run -p miscord-server

# Terminal 2 - Start client for Bob
MISCORD_PROFILE=bob MISCORD_WINDOW_TITLE="Miscord - Bob" cargo run -p miscord-client

# Terminal 3 - Start client for Alice
MISCORD_PROFILE=alice MISCORD_WINDOW_TITLE="Miscord - Alice" cargo run -p miscord-client
```

Each profile stores its session in a separate file under:
`~/Library/Application Support/miscord/sessions/<profile>.json` (macOS)

**Important:** Without different profiles, both clients will share the same session and log in as the same user.

## Testing Message Flow

1. Launch two clients (Alice and Bob)
2. Both clients will auto-login and see the "Dev Server"
3. Select the server and go to the #general channel
4. Type messages in one client and see them appear in the other

## Testing Video/Screen Sharing

1. Start the server and two clients with different profiles (see above)
2. Log in as different users (bob/alice with password `password123`)
3. Both users join the same voice channel
4. Click the screen share button on one client
5. Select a monitor and resolution
6. On the other client, click "Watch" on the screen share tile

## Stopping the Environment

- Press `Ctrl+C` in the server terminal to stop the server
- Run `docker-compose down` to stop PostgreSQL
- Run `docker-compose down -v` to also remove the database volume (clean slate)

## Troubleshooting

### "Docker is not running"

Start Docker Desktop or run `colima start` if using Colima.

### "Connection refused" errors

Make sure the server is running and PostgreSQL is healthy:

```bash
docker-compose ps
curl http://localhost:8080/health
```

### Database migration errors

Reset the database and try again:

```bash
docker-compose down -v
docker-compose up -d postgres
./scripts/dev-server.sh
```

### Client won't connect

1. Check the server is running at http://localhost:8080
2. Verify the MISCORD_SERVER_URL environment variable
3. Check the server logs for errors

### Screen share stuck at "Loading..."

This can happen when the WebRTC renegotiation fails. Common causes:

#### 1. Codec capability mismatch (fixed in Jan 2026)

**Symptom:** Server logs show `Failed to create offer for renegotiation: unable to populate media section, RTPSender created with no codecs`

**Cause:** When creating a `TrackLocalStaticRTP` for forwarding video to subscribers, the codec capability must exactly match what's registered in the server's `MediaEngine`. Using the source track's codec capability doesn't work because it may have different fmtp parameters.

**Fix:** In `track_router.rs`, the `add_subscriber()` method uses a hardcoded H.264 codec capability that matches the `MediaEngine` registration:

```rust
// Use hardcoded H.264 capability matching the MediaEngine registration
let h264_capability = RTCRtpCodecCapability {
    mime_type: "video/H264".to_string(),
    clock_rate: 90000,
    channels: 0,
    sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_string(),
    rtcp_feedback: vec![],
};
```

**Key insight:** The codec capability in `TrackLocalStaticRTP::new()` must match the codec registered in `SfuSessionManager::new()` (in `session.rs`). If you change one, you must change the other.

#### 2. RTP receiver not ready

**Symptom:** Server logs show `Error reading RTP from source track: RTPReceiver must not be nil`

**Cause:** The track router starts forwarding before the WebRTC RTP receiver is fully initialized.

**Fix:** The track router now includes retry logic that waits up to 5 seconds for the receiver to become ready before starting to forward packets.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                         Clients                              │
│  ┌──────────────────┐         ┌──────────────────┐          │
│  │  Alice (egui)    │         │  Bob (egui)      │          │
│  │  Port: random    │         │  Port: random    │          │
│  └────────┬─────────┘         └────────┬─────────┘          │
│           │                            │                     │
│           └────────────┬───────────────┘                     │
│                        │                                     │
│                        ▼                                     │
│  ┌─────────────────────────────────────────────────────┐    │
│  │              Miscord Server (Axum)                   │    │
│  │              http://localhost:8080                   │    │
│  │                                                      │    │
│  │  REST API: /api/*    WebSocket: /ws                 │    │
│  └────────────────────────┬────────────────────────────┘    │
│                           │                                  │
│                           ▼                                  │
│  ┌─────────────────────────────────────────────────────┐    │
│  │              PostgreSQL (Docker)                     │    │
│  │              localhost:5434                          │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```
