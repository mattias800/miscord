#!/bin/bash
# Launch Miscord client as Alice

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

export MISCORD_SERVER_URL="http://localhost:8080"
export MISCORD_AUTO_LOGIN_USER="alice"
export MISCORD_AUTO_LOGIN_PASS="password123"
export MISCORD_WINDOW_TITLE="Miscord - Alice"

echo "Starting Miscord client as Alice..."
cargo run -p miscord-client
