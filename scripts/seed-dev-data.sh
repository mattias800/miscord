#!/bin/bash
# Seed development data via API
# Creates test users and a test server

set -e

API_URL="${MISCORD_API_URL:-http://localhost:8080}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}=== Seeding Miscord Development Data ===${NC}"
echo -e "API URL: ${API_URL}"
echo ""

# Wait for server to be ready
echo -e "${YELLOW}Waiting for server to be ready...${NC}"
for i in {1..30}; do
    if curl -s "${API_URL}/health" > /dev/null 2>&1; then
        echo -e "${GREEN}Server is ready!${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Timeout waiting for server. Is it running?${NC}"
        exit 1
    fi
    sleep 1
done

echo ""

# Register Alice
echo -e "${YELLOW}Registering user: alice${NC}"
ALICE_RESULT=$(curl -s -X POST "${API_URL}/api/auth/register" \
    -H "Content-Type: application/json" \
    -d '{
        "username": "alice",
        "display_name": "Alice Developer",
        "email": "alice@example.com",
        "password": "password123"
    }' 2>&1)

if echo "$ALICE_RESULT" | grep -q "user_id\|already exists"; then
    echo -e "${GREEN}Alice: OK${NC}"
else
    echo -e "${RED}Alice: Failed - $ALICE_RESULT${NC}"
fi

# Register Bob
echo -e "${YELLOW}Registering user: bob${NC}"
BOB_RESULT=$(curl -s -X POST "${API_URL}/api/auth/register" \
    -H "Content-Type: application/json" \
    -d '{
        "username": "bob",
        "display_name": "Bob Tester",
        "email": "bob@example.com",
        "password": "password123"
    }' 2>&1)

if echo "$BOB_RESULT" | grep -q "user_id\|already exists"; then
    echo -e "${GREEN}Bob: OK${NC}"
else
    echo -e "${RED}Bob: Failed - $BOB_RESULT${NC}"
fi

# Register Charlie
echo -e "${YELLOW}Registering user: charlie${NC}"
CHARLIE_RESULT=$(curl -s -X POST "${API_URL}/api/auth/register" \
    -H "Content-Type: application/json" \
    -d '{
        "username": "charlie",
        "display_name": "Charlie Observer",
        "email": "charlie@example.com",
        "password": "password123"
    }' 2>&1)

if echo "$CHARLIE_RESULT" | grep -q "user_id\|already exists"; then
    echo -e "${GREEN}Charlie: OK${NC}"
else
    echo -e "${RED}Charlie: Failed - $CHARLIE_RESULT${NC}"
fi

echo ""

# Login as Alice to create a server
echo -e "${YELLOW}Logging in as Alice to create server...${NC}"
ALICE_LOGIN=$(curl -s -X POST "${API_URL}/api/auth/login" \
    -H "Content-Type: application/json" \
    -d '{
        "username": "alice",
        "password": "password123"
    }')

ALICE_TOKEN=$(echo "$ALICE_LOGIN" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)

if [ -n "$ALICE_TOKEN" ]; then
    echo -e "${GREEN}Alice logged in successfully${NC}"

    # Create a server
    echo -e "${YELLOW}Creating Dev Server...${NC}"
    SERVER_RESULT=$(curl -s -X POST "${API_URL}/api/servers" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${ALICE_TOKEN}" \
        -d '{
            "name": "Dev Server",
            "description": "A server for development and testing"
        }')

    SERVER_ID=$(echo "$SERVER_RESULT" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)

    if [ -n "$SERVER_ID" ]; then
        echo -e "${GREEN}Server created: ${SERVER_ID}${NC}"

        # Create an invite
        echo -e "${YELLOW}Creating invite...${NC}"
        INVITE_RESULT=$(curl -s -X POST "${API_URL}/api/servers/${SERVER_ID}/invites" \
            -H "Authorization: Bearer ${ALICE_TOKEN}")

        INVITE_CODE=$(echo "$INVITE_RESULT" | grep -o '"code":"[^"]*"' | cut -d'"' -f4)

        if [ -n "$INVITE_CODE" ]; then
            echo -e "${GREEN}Invite code: ${INVITE_CODE}${NC}"

            # Login as Bob and join the server
            echo -e "${YELLOW}Bob joining the server...${NC}"
            BOB_LOGIN=$(curl -s -X POST "${API_URL}/api/auth/login" \
                -H "Content-Type: application/json" \
                -d '{
                    "username": "bob",
                    "password": "password123"
                }')

            BOB_TOKEN=$(echo "$BOB_LOGIN" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)

            if [ -n "$BOB_TOKEN" ]; then
                curl -s -X POST "${API_URL}/api/invites/${INVITE_CODE}" \
                    -H "Authorization: Bearer ${BOB_TOKEN}" > /dev/null
                echo -e "${GREEN}Bob joined the server${NC}"
            fi

            # Login as Charlie and join the server
            echo -e "${YELLOW}Charlie joining the server...${NC}"
            CHARLIE_LOGIN=$(curl -s -X POST "${API_URL}/api/auth/login" \
                -H "Content-Type: application/json" \
                -d '{
                    "username": "charlie",
                    "password": "password123"
                }')

            CHARLIE_TOKEN=$(echo "$CHARLIE_LOGIN" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)

            if [ -n "$CHARLIE_TOKEN" ]; then
                curl -s -X POST "${API_URL}/api/invites/${INVITE_CODE}" \
                    -H "Authorization: Bearer ${CHARLIE_TOKEN}" > /dev/null
                echo -e "${GREEN}Charlie joined the server${NC}"
            fi
        fi
    else
        echo -e "${YELLOW}Server may already exist${NC}"
    fi
else
    echo -e "${RED}Failed to login as Alice${NC}"
fi

echo ""
echo -e "${GREEN}=== Seed Complete ===${NC}"
echo ""
echo -e "Test users created with password: ${BLUE}password123${NC}"
echo ""
echo -e "  Username     Display Name"
echo -e "  ${BLUE}alice${NC}        Alice Developer"
echo -e "  ${BLUE}bob${NC}          Bob Tester"
echo -e "  ${BLUE}charlie${NC}      Charlie Observer"
echo ""
