#!/bin/bash
# Development server startup script
# Starts PostgreSQL and the Miscord server

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== Miscord Development Server ===${NC}"

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not running. Please start Docker first.${NC}"
    exit 1
fi

# Start PostgreSQL
echo -e "${YELLOW}Starting PostgreSQL...${NC}"
docker-compose up -d postgres

# Wait for PostgreSQL to be ready
echo -e "${YELLOW}Waiting for PostgreSQL to be ready...${NC}"
for i in {1..30}; do
    if docker-compose exec -T postgres pg_isready -U miscord > /dev/null 2>&1; then
        echo -e "${GREEN}PostgreSQL is ready!${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Timeout waiting for PostgreSQL${NC}"
        exit 1
    fi
    sleep 1
done

# Set environment variables
export DATABASE_URL="postgres://miscord:miscord@localhost:5434/miscord"
export JWT_SECRET="dev-secret-do-not-use-in-production"
export BIND_ADDRESS="0.0.0.0:8080"
export RUST_LOG="miscord_server=debug,tower_http=debug"

# Build and run the server
echo -e "${YELLOW}Building and starting Miscord server...${NC}"
echo -e "${BLUE}Server will be available at http://localhost:8080${NC}"
echo ""

cargo run -p miscord-server
