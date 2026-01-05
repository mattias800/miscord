#!/bin/bash
# Launch Miscord client as Bob

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

export MISCORD_SERVER_URL="http://localhost:8080"
export MISCORD_AUTO_LOGIN_USER="bob"
export MISCORD_AUTO_LOGIN_PASS="password123"
export MISCORD_WINDOW_TITLE="Miscord - Bob"

echo "Starting Miscord client as Bob..."
cargo run -p miscord-client
