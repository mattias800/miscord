# Agent Guidelines for Miscord

This document contains important notes and guidelines for AI agents working on the Miscord codebase.

## IMPORTANT: Process Management

**NEVER use `pkill -f miscord` or similar broad process killing commands.**

The user may have other Miscord-related processes running (such as a C# backend, multiple clients for testing, etc.) that should not be killed. Using broad pattern matching with pkill will terminate these processes unintentionally.

Instead:
- Ask the user if processes need to be stopped
- Use specific PIDs when killing processes
- Only kill processes that were started in the current session
- When starting clients, check if the server is already running first rather than assuming you need to restart everything

## Client Configuration

The Miscord client uses environment variables for configuration:

- `MISCORD_WINDOW_TITLE` - Window title (e.g., "Miscord - Bob")
- `MISCORD_PROFILE` - Profile name for session storage (e.g., "bob", "alice")
- `MISCORD_AUTO_LOGIN_USER` - Username for auto-login
- `MISCORD_AUTO_LOGIN_PASS` - Password for auto-login
- `MISCORD_SERVER_URL` - Server URL to connect to

Example starting two clients:
```bash
# Start Bob's client
MISCORD_WINDOW_TITLE="Miscord - Bob" MISCORD_PROFILE="bob" MISCORD_AUTO_LOGIN_USER="bob" MISCORD_AUTO_LOGIN_PASS="password123" cargo run --release --bin miscord &

# Start Alice's client
MISCORD_WINDOW_TITLE="Miscord - Alice" MISCORD_PROFILE="alice" MISCORD_AUTO_LOGIN_USER="alice" MISCORD_AUTO_LOGIN_PASS="password123" cargo run --release --bin miscord &
```
