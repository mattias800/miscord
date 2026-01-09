#!/bin/bash

# Development startup script - starts server and two clients with different test accounts
# Usage: ./dev-start.sh
#
# This script will:
# 1. Start PostgreSQL (via docker-compose)
# 2. Build and start the server
# 3. Seed test data (creates alice, bob, charlie accounts)
# 4. Build and start two clients (Alice and Bob) with auto-login

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

SERVER_URL="http://localhost:8080"

# Test accounts (password: password123)
USER1_NAME="alice"
USER1_PASS="password123"
USER2_NAME="bob"
USER2_PASS="password123"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}╔══════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Miscord Development Startup        ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════╝${NC}"
echo ""

# Function to find and kill existing miscord processes safely
cleanup_existing() {
    echo -e "${YELLOW}Checking for existing Miscord processes...${NC}"

    # Find miscord-server processes
    local server_pids=$(ps aux | grep "target/debug/miscord-server" | grep -v grep | awk '{print $2}')
    if [ -n "$server_pids" ]; then
        echo -e "  Stopping existing server(s): $server_pids"
        for pid in $server_pids; do
            kill "$pid" 2>/dev/null || true
        done
    fi

    # Find miscord (client) processes - but be careful not to kill other things
    local client_pids=$(ps aux | grep "target/debug/miscord[^-]" | grep -v grep | awk '{print $2}')
    if [ -n "$client_pids" ]; then
        echo -e "  Stopping existing client(s): $client_pids"
        for pid in $client_pids; do
            kill "$pid" 2>/dev/null || true
        done
    fi

    sleep 1
}

# Cleanup on exit
cleanup() {
    echo ""
    echo -e "${YELLOW}Shutting down...${NC}"
    [ -n "$CLIENT1_PID" ] && kill -9 $CLIENT1_PID 2>/dev/null || true
    [ -n "$CLIENT2_PID" ] && kill -9 $CLIENT2_PID 2>/dev/null || true
    [ -n "$SERVER_PID" ] && kill -9 $SERVER_PID 2>/dev/null || true
    wait 2>/dev/null || true
    echo -e "${GREEN}All processes stopped.${NC}"
    exit 0
}

trap cleanup SIGINT SIGTERM

# Stop any existing processes
cleanup_existing

# Check if Docker is running
echo -e "${YELLOW}[1/6] Checking Docker...${NC}"
if ! docker info > /dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not running. Please start Docker first.${NC}"
    echo -e "  On macOS with Colima: ${BLUE}colima start${NC}"
    echo -e "  Or start Docker Desktop"
    exit 1
fi
echo -e "${GREEN}      Docker is running${NC}"

# Start PostgreSQL
echo -e "${YELLOW}[2/6] Starting PostgreSQL...${NC}"
docker-compose up -d postgres

# Wait for PostgreSQL to be ready
for i in {1..30}; do
    if docker-compose exec -T postgres pg_isready -U miscord > /dev/null 2>&1; then
        echo -e "${GREEN}      PostgreSQL is ready${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Timeout waiting for PostgreSQL${NC}"
        exit 1
    fi
    sleep 1
done

# Build projects
echo -e "${YELLOW}[3/6] Building projects...${NC}"
cargo build -p miscord-server -p miscord-client --quiet
echo -e "${GREEN}      Build complete${NC}"

# Set environment variables for server
export DATABASE_URL="postgres://miscord:miscord@localhost:5434/miscord"
export JWT_SECRET="dev-secret-do-not-use-in-production"
export BIND_ADDRESS="0.0.0.0:8080"
export RUST_LOG="miscord_server=info,tower_http=warn"

# Start server in background
echo -e "${YELLOW}[4/6] Starting server...${NC}"
cargo run -p miscord-server --quiet &
SERVER_PID=$!
echo -e "      Server PID: $SERVER_PID"

# Wait for server to be ready
echo -e "      Waiting for server..."
for i in {1..30}; do
    if curl -s "$SERVER_URL/api/health" > /dev/null 2>&1; then
        echo -e "${GREEN}      Server is ready at $SERVER_URL${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Server failed to start!${NC}"
        kill $SERVER_PID 2>/dev/null
        exit 1
    fi
    sleep 1
done

# Seed development data (creates test users)
echo -e "${YELLOW}[5/6] Seeding test data...${NC}"
if [ -f "./scripts/seed-dev-data.sh" ]; then
    ./scripts/seed-dev-data.sh > /dev/null 2>&1 || true
    echo -e "${GREEN}      Test users ready${NC}"
else
    echo -e "${YELLOW}      Seed script not found, skipping${NC}"
fi

# Start clients
echo -e "${YELLOW}[6/6] Starting clients...${NC}"

# Client 1 - Alice
RUST_BACKTRACE=1 \
RUST_LOG=miscord_client=info \
MISCORD_PROFILE=alice \
MISCORD_WINDOW_TITLE="Miscord - Alice" \
MISCORD_AUTO_LOGIN_USER="$USER1_NAME" \
MISCORD_AUTO_LOGIN_PASS="$USER1_PASS" \
cargo run -p miscord-client --quiet 2>&1 | tee /tmp/miscord_alice.log &
CLIENT1_PID=$!
echo -e "      Client 1 (Alice) PID: $CLIENT1_PID"
echo -e "      Log: /tmp/miscord_alice.log"

sleep 2

# Client 2 - Bob
RUST_BACKTRACE=1 \
RUST_LOG=miscord_client=info \
MISCORD_PROFILE=bob \
MISCORD_WINDOW_TITLE="Miscord - Bob" \
MISCORD_AUTO_LOGIN_USER="$USER2_NAME" \
MISCORD_AUTO_LOGIN_PASS="$USER2_PASS" \
cargo run -p miscord-client --quiet 2>&1 | tee /tmp/miscord_bob.log &
CLIENT2_PID=$!
echo -e "      Client 2 (Bob) PID: $CLIENT2_PID"
echo -e "      Log: /tmp/miscord_bob.log"

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║         All Processes Started!           ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Server:   PID $SERVER_PID  ($SERVER_URL)"
echo -e "  Client 1: PID $CLIENT1_PID  (Alice)"
echo -e "  Client 2: PID $CLIENT2_PID  (Bob)"
echo ""
echo -e "Test accounts use password: ${BLUE}password123${NC}"
echo ""
echo -e "Press ${RED}Ctrl+C${NC} to stop all processes"
echo ""

# Wait indefinitely (trap will handle Ctrl+C)
while true; do
    sleep 1
done
