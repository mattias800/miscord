//! Integration tests for Miscord server
//!
//! These tests require a running PostgreSQL database.
//! Set TEST_DATABASE_URL environment variable to configure.
//!
//! Run with: cargo test -p miscord-server --test integration_tests

use futures_util::{SinkExt, StreamExt};
use miscord_protocol::{ClientMessage, ServerMessage};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Test helper to start a test server
async fn start_test_server() -> TestServer {
    TestServer::start().await.expect("Failed to start test server")
}

/// Test server wrapper
struct TestServer {
    addr: std::net::SocketAddr,
    #[allow(dead_code)]
    db_pool: sqlx::PgPool,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TestServer {
    async fn start() -> anyhow::Result<Self> {
        // Use test database URL from environment or default
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://miscord:miscord@localhost:5434/miscord_test".to_string());

        let config = miscord_server::state::Config {
            database_url,
            jwt_secret: "test-secret-key-for-testing-only".to_string(),
            bind_address: "127.0.0.1:0".to_string(),
            stun_servers: vec![],
            turn_servers: vec![],
        };

        let (router, db_pool) = miscord_server::create_app(config).await?;

        // Bind to random port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        // Spawn server
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .ok();
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(Self {
            addr,
            db_pool,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    fn http_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn ws_url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Create a test user and return their auth token
async fn create_test_user(
    client: &Client,
    http_url: &str,
    username: &str,
) -> anyhow::Result<(String, uuid::Uuid)> {
    let password = "testpassword123";

    // Try to register (might fail if user exists)
    let _ = client
        .post(format!("{}/api/auth/register", http_url))
        .json(&json!({
            "username": username,
            "email": format!("{}@test.com", username),
            "password": password,
            "display_name": username
        }))
        .send()
        .await;

    // Login
    let login_response = client
        .post(format!("{}/api/auth/login", http_url))
        .json(&json!({
            "username": username,
            "password": password
        }))
        .send()
        .await?;

    let login_data: serde_json::Value = login_response.json().await?;

    let token = login_data["token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No token in response"))?
        .to_string();

    let user_id = login_data["user_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| anyhow::anyhow!("No user_id in response"))?;

    Ok((token, user_id))
}

/// Create a server and return its ID
async fn create_server(
    client: &Client,
    http_url: &str,
    token: &str,
    name: &str,
) -> anyhow::Result<uuid::Uuid> {
    let response = client
        .post(format!("{}/api/servers", http_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({ "name": name }))
        .send()
        .await?;

    let data: serde_json::Value = response.json().await?;
    let server_id = data["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| anyhow::anyhow!("No server id in response"))?;

    Ok(server_id)
}

/// Get channels for a server
async fn get_channels(
    client: &Client,
    http_url: &str,
    token: &str,
    server_id: uuid::Uuid,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let response = client
        .get(format!("{}/api/servers/{}/channels", http_url, server_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    let channels: Vec<serde_json::Value> = response.json().await?;
    Ok(channels)
}

/// Connect to WebSocket and authenticate
async fn connect_websocket(
    ws_url: &str,
    token: &str,
) -> anyhow::Result<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
> {
    let (ws_stream, _) = connect_async(ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    // Send authentication
    let auth_msg = ClientMessage::Authenticate {
        token: token.to_string(),
    };
    write
        .send(Message::Text(serde_json::to_string(&auth_msg)?.into()))
        .await?;

    // Wait for authentication response
    let response = timeout(Duration::from_secs(5), read.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("No response"))??;

    if let Message::Text(text) = response {
        let msg: ServerMessage = serde_json::from_str(&text)?;
        match msg {
            ServerMessage::Authenticated { .. } => {
                // Reunite the stream
                Ok(write.reunite(read)?)
            }
            ServerMessage::Error { message } => {
                Err(anyhow::anyhow!("Auth failed: {}", message))
            }
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    } else {
        Err(anyhow::anyhow!("Expected text message"))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_user_registration_and_login() {
    let server = start_test_server().await;
    let client = Client::new();
    let username = format!("testuser_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

    // Register
    let register_response = client
        .post(format!("{}/api/auth/register", server.http_url()))
        .json(&json!({
            "username": &username,
            "email": format!("{}@test.com", username),
            "password": "testpassword123",
            "display_name": &username
        }))
        .send()
        .await
        .expect("Register request failed");

    assert!(
        register_response.status().is_success(),
        "Registration failed: {}",
        register_response.text().await.unwrap_or_default()
    );

    // Login
    let login_response = client
        .post(format!("{}/api/auth/login", server.http_url()))
        .json(&json!({
            "username": &username,
            "password": "testpassword123"
        }))
        .send()
        .await
        .expect("Login request failed");

    assert!(login_response.status().is_success());

    let login_data: serde_json::Value = login_response.json().await.unwrap();
    assert!(login_data["token"].is_string());
    assert!(login_data["user_id"].is_string());
}

#[tokio::test]
async fn test_create_server_and_channels() {
    let server = start_test_server().await;
    let client = Client::new();
    let username = format!("testuser_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

    let (token, _user_id) = create_test_user(&client, &server.http_url(), &username)
        .await
        .expect("Failed to create test user");

    // Create server
    let server_id = create_server(&client, &server.http_url(), &token, "Test Server")
        .await
        .expect("Failed to create server");

    // Get channels (should have default #general)
    let channels = get_channels(&client, &server.http_url(), &token, server_id)
        .await
        .expect("Failed to get channels");

    assert!(!channels.is_empty(), "Server should have default channels");

    // Find text channel
    let text_channel = channels.iter().find(|c| c["channel_type"] == "text");
    assert!(text_channel.is_some(), "Should have a text channel");
}

#[tokio::test]
async fn test_websocket_connection() {
    let server = start_test_server().await;
    let client = Client::new();
    let username = format!("testuser_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

    let (token, _user_id) = create_test_user(&client, &server.http_url(), &username)
        .await
        .expect("Failed to create test user");

    // Connect WebSocket
    let ws_stream = connect_websocket(&server.ws_url(), &token)
        .await
        .expect("Failed to connect WebSocket");

    // Connection successful if we get here
    drop(ws_stream);
}

#[tokio::test]
async fn test_realtime_message_delivery() {
    let server = start_test_server().await;
    let client = Client::new();

    // Create two users: Alice and Bob
    let alice_username = format!("alice_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());
    let bob_username = format!("bob_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

    let (alice_token, _alice_id) = create_test_user(&client, &server.http_url(), &alice_username)
        .await
        .expect("Failed to create Alice");

    let (bob_token, _bob_id) = create_test_user(&client, &server.http_url(), &bob_username)
        .await
        .expect("Failed to create Bob");

    // Alice creates a server
    let server_id = create_server(&client, &server.http_url(), &alice_token, "Test Server")
        .await
        .expect("Failed to create server");

    // Get the general channel
    let channels = get_channels(&client, &server.http_url(), &alice_token, server_id)
        .await
        .expect("Failed to get channels");

    let channel = channels
        .iter()
        .find(|c| c["channel_type"] == "text")
        .expect("No text channel found");

    let channel_id: uuid::Uuid = channel["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .expect("Invalid channel id");

    // Bob joins the server via invite
    // First, Alice creates an invite
    let invite_response = client
        .post(format!("{}/api/servers/{}/invites", server.http_url(), server_id))
        .header("Authorization", format!("Bearer {}", alice_token))
        .send()
        .await
        .expect("Failed to create invite");

    let invite_data: serde_json::Value = invite_response.json().await.unwrap();
    let invite_code = invite_data["code"].as_str().expect("No invite code");

    // Bob uses the invite
    client
        .post(format!("{}/api/invites/{}", server.http_url(), invite_code))
        .header("Authorization", format!("Bearer {}", bob_token))
        .send()
        .await
        .expect("Failed to join server");

    // Connect both users to WebSocket
    let alice_ws = connect_websocket(&server.ws_url(), &alice_token)
        .await
        .expect("Alice failed to connect");

    let bob_ws = connect_websocket(&server.ws_url(), &bob_token)
        .await
        .expect("Bob failed to connect");

    let (mut alice_write, mut alice_read) = alice_ws.split();
    let (mut bob_write, mut bob_read) = bob_ws.split();

    // Both subscribe to the channel
    let subscribe_msg = ClientMessage::SubscribeChannel { channel_id };
    alice_write
        .send(Message::Text(serde_json::to_string(&subscribe_msg).unwrap().into()))
        .await
        .unwrap();
    bob_write
        .send(Message::Text(serde_json::to_string(&subscribe_msg).unwrap().into()))
        .await
        .unwrap();

    // Wait for subscription confirmations
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drain any pending messages
    while let Ok(Some(_)) = timeout(Duration::from_millis(50), alice_read.next()).await {}
    while let Ok(Some(_)) = timeout(Duration::from_millis(50), bob_read.next()).await {}

    // Bob sends a message via REST API
    let message_content = format!("Hello from Bob! {}", uuid::Uuid::new_v4());
    let send_response = client
        .post(format!("{}/api/channels/{}/messages", server.http_url(), channel_id))
        .header("Authorization", format!("Bearer {}", bob_token))
        .json(&json!({ "content": &message_content }))
        .send()
        .await
        .expect("Failed to send message");

    assert!(send_response.status().is_success(), "Message send failed");

    // Alice should receive the message via WebSocket
    let alice_message = timeout(Duration::from_secs(5), alice_read.next())
        .await
        .expect("Timeout waiting for message")
        .expect("Stream ended")
        .expect("WebSocket error");

    if let Message::Text(text) = alice_message {
        let msg: ServerMessage = serde_json::from_str(&text).expect("Failed to parse message");

        match msg {
            ServerMessage::MessageCreated { message } => {
                assert_eq!(
                    message.content, message_content,
                    "Message content doesn't match"
                );
                assert_eq!(
                    message.channel_id, channel_id,
                    "Channel ID doesn't match"
                );
            }
            other => panic!("Expected MessageCreated, got {:?}", other),
        }
    } else {
        panic!("Expected text message");
    }
}

#[tokio::test]
async fn test_send_message_via_api() {
    let server = start_test_server().await;
    let client = Client::new();
    let username = format!("testuser_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

    let (token, _user_id) = create_test_user(&client, &server.http_url(), &username)
        .await
        .expect("Failed to create test user");

    // Create server
    let server_id = create_server(&client, &server.http_url(), &token, "Test Server")
        .await
        .expect("Failed to create server");

    // Get channels
    let channels = get_channels(&client, &server.http_url(), &token, server_id)
        .await
        .expect("Failed to get channels");

    let channel = channels
        .iter()
        .find(|c| c["channel_type"] == "text")
        .expect("No text channel found");

    let channel_id = channel["id"].as_str().expect("No channel id");

    // Send message
    let message_content = "Hello, world!";
    let response = client
        .post(format!("{}/api/channels/{}/messages", server.http_url(), channel_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({ "content": message_content }))
        .send()
        .await
        .expect("Failed to send message");

    assert!(response.status().is_success());

    let message_data: serde_json::Value = response.json().await.unwrap();
    assert_eq!(message_data["content"], message_content);

    // Get messages
    let messages_response = client
        .get(format!("{}/api/channels/{}/messages", server.http_url(), channel_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("Failed to get messages");

    let messages: Vec<serde_json::Value> = messages_response.json().await.unwrap();
    assert!(!messages.is_empty());
    assert!(messages.iter().any(|m| m["content"] == message_content));
}
