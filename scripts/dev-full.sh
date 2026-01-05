#!/bin/bash
# Full development environment startup
# Starts server, seeds data, and launches two clients

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}╔══════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     Miscord Full Development Setup       ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════╝${NC}"
echo ""

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not running. Please start Docker first.${NC}"
    exit 1
fi

# Start PostgreSQL
echo -e "${YELLOW}[1/5] Starting PostgreSQL...${NC}"
docker-compose up -d postgres

# Wait for PostgreSQL to be ready
echo -e "${YELLOW}[2/5] Waiting for PostgreSQL...${NC}"
for i in {1..30}; do
    if docker-compose exec -T postgres pg_isready -U miscord > /dev/null 2>&1; then
        echo -e "${GREEN}      PostgreSQL is ready!${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Timeout waiting for PostgreSQL${NC}"
        exit 1
    fi
    sleep 1
done

# Set environment variables for server
export DATABASE_URL="postgres://miscord:miscord@localhost:5434/miscord"
export JWT_SECRET="dev-secret-do-not-use-in-production"
export BIND_ADDRESS="0.0.0.0:8080"
export RUST_LOG="miscord_server=info,tower_http=info"

# Start server in background
echo -e "${YELLOW}[3/5] Starting Miscord server...${NC}"
cargo build -p miscord-server --quiet
cargo run -p miscord-server --quiet &
SERVER_PID=$!

# Wait for server to be ready
sleep 3
for i in {1..30}; do
    if curl -s "http://localhost:8080/health" > /dev/null 2>&1; then
        echo -e "${GREEN}      Server is ready at http://localhost:8080${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Timeout waiting for server${NC}"
        kill $SERVER_PID 2>/dev/null
        exit 1
    fi
    sleep 1
done

# Seed development data
echo -e "${YELLOW}[4/5] Seeding development data...${NC}"
"$SCRIPT_DIR/seed-dev-data.sh" > /dev/null 2>&1
echo -e "${GREEN}      Test users created${NC}"

# Build client
echo -e "${YELLOW}[5/5] Building client...${NC}"
cargo build -p miscord-client --quiet
echo -e "${GREEN}      Client built${NC}"

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║          Development Ready!              ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════╝${NC}"
echo ""
echo -e "Server running at: ${BLUE}http://localhost:8080${NC}"
echo -e "Server PID: ${SERVER_PID}"
echo ""
echo -e "Test accounts (password: ${BLUE}password123${NC}):"
echo -e "  • alice  - Alice Developer"
echo -e "  • bob    - Bob Tester"
echo -e "  • charlie - Charlie Observer"
echo ""
echo -e "${YELLOW}To launch clients:${NC}"
echo -e "  Terminal 1: ${BLUE}./scripts/client-alice.sh${NC}"
echo -e "  Terminal 2: ${BLUE}./scripts/client-bob.sh${NC}"
echo ""
echo -e "${YELLOW}Or run manually:${NC}"
echo -e "  ${BLUE}cargo run -p miscord-client${NC}"
echo ""
echo -e "Press ${RED}Ctrl+C${NC} to stop the server"
echo ""

# Wait for server
wait $SERVER_PID
