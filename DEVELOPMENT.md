# Miscord Development Guide

This guide explains how to set up and run Miscord for development and testing.

## Prerequisites

- Rust (latest stable)
- Docker and Docker Compose
- A terminal that supports ANSI colors (optional, for pretty output)

## Quick Start

### Option 1: Full Development Environment (Recommended)

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
| MISCORD_AUTO_LOGIN_USER | (none)              | Username for auto-login          |
| MISCORD_AUTO_LOGIN_PASS | (none)              | Password for auto-login          |

## Testing Message Flow

1. Launch two clients (Alice and Bob)
2. Both clients will auto-login and see the "Dev Server"
3. Select the server and go to the #general channel
4. Type messages in one client and see them appear in the other

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
