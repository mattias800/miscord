//! Automated SFU video streaming test
//!
//! This test simulates two users joining a voice channel with video enabled
//! and verifies that video frames are being exchanged.

use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

use miscord_client::network::NetworkClient;
use miscord_client::media::sfu_client::{SfuClient, IceCandidate};
use miscord_client::media::gst_video::VideoFrame;
use tokio::sync::mpsc;

const SERVER_URL: &str = "http://localhost:3000";
const WS_URL: &str = "ws://localhost:3000/ws";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,miscord=debug")
        .init();

    println!("=== SFU Video Streaming Test ===\n");

    // Create two network clients
    println!("1. Creating two users...");

    let bob_client = NetworkClient::new(SERVER_URL);
    let alice_client = NetworkClient::new(SERVER_URL);

    // Login or register users
    println!("   Logging in as Bob...");
    let bob_token = match bob_client.login("bob", "password123").await {
        Ok(token) => token,
        Err(_) => {
            println!("   Bob doesn't exist, registering...");
            bob_client.register("bob", "bob@test.com", "password123").await?
        }
    };
    bob_client.set_token(bob_token.clone());

    println!("   Logging in as Alice...");
    let alice_token = match alice_client.login("alice", "password123").await {
        Ok(token) => token,
        Err(_) => {
            println!("   Alice doesn't exist, registering...");
            alice_client.register("alice", "alice@test.com", "password123").await?
        }
    };
    alice_client.set_token(alice_token.clone());

    println!("   ✓ Both users logged in\n");

    // Get or create a community and voice channel
    println!("2. Getting test community and voice channel...");
    let communities = bob_client.get_communities().await?;
    let community = communities.first().ok_or_else(|| anyhow::anyhow!("No communities found"))?;
    println!("   Using community: {}", community.name);

    // Find a voice channel
    let channels = bob_client.get_channels(community.id).await?;
    let voice_channel = channels.iter()
        .find(|c| c.channel_type == "voice")
        .ok_or_else(|| anyhow::anyhow!("No voice channel found"))?;
    println!("   Using voice channel: {} ({})", voice_channel.name, voice_channel.id);
    println!();

    // Connect WebSocket for both users
    println!("3. Connecting WebSockets...");
    bob_client.connect_websocket(WS_URL).await?;
    alice_client.connect_websocket(WS_URL).await?;
    println!("   ✓ WebSockets connected\n");

    // Join voice channel
    println!("4. Joining voice channel...");
    bob_client.join_voice(voice_channel.id).await?;
    sleep(Duration::from_millis(500)).await;
    alice_client.join_voice(voice_channel.id).await?;
    println!("   ✓ Both users in voice channel\n");

    // Enable video for Bob
    println!("5. Enabling video for Bob...");
    bob_client.update_voice_state(None, None, Some(true), None).await?;
    sleep(Duration::from_millis(500)).await;
    println!("   ✓ Bob's video enabled\n");

    // Create SFU clients
    println!("6. Creating SFU connections...");

    let (bob_ice_tx, mut bob_ice_rx) = mpsc::unbounded_channel::<IceCandidate>();
    let (alice_ice_tx, mut alice_ice_rx) = mpsc::unbounded_channel::<IceCandidate>();

    let bob_sfu = SfuClient::new();
    let alice_sfu = SfuClient::new();

    // Bob connects to SFU (he has video enabled)
    println!("   Bob connecting to SFU...");
    let ice_servers = vec![("stun:stun.l.google.com:19302".to_string(), None, None)];
    let bob_offer = bob_sfu.connect(voice_channel.id, ice_servers.clone(), bob_ice_tx).await?;
    println!("   Bob SFU offer created ({} bytes)", bob_offer.len());

    // Send Bob's offer to server
    bob_client.send_sfu_offer(voice_channel.id, bob_offer).await;
    println!("   Bob's offer sent to server");

    // Wait for answer
    sleep(Duration::from_secs(2)).await;

    // Alice connects to SFU
    println!("   Alice connecting to SFU...");
    let alice_offer = alice_sfu.connect(voice_channel.id, ice_servers.clone(), alice_ice_tx).await?;
    println!("   Alice SFU offer created ({} bytes)", alice_offer.len());

    alice_client.send_sfu_offer(voice_channel.id, alice_offer).await;
    println!("   Alice's offer sent to server");

    // Wait for connections to establish
    println!("\n7. Waiting for connections to establish...");
    sleep(Duration::from_secs(3)).await;

    // Check connection status
    println!("   Bob SFU connected: {}", bob_sfu.is_connected().await);
    println!("   Alice SFU connected: {}", alice_sfu.is_connected().await);

    // Send test frames from Bob
    println!("\n8. Sending test video frames from Bob...");
    let test_frame = VideoFrame {
        width: 640,
        height: 480,
        data: vec![128u8; 640 * 480 * 3], // Gray frame
    };

    for i in 0..30 {
        if let Err(e) = bob_sfu.send_frame(&test_frame).await {
            println!("   Frame {} error: {}", i, e);
        } else if i % 10 == 0 {
            println!("   Sent frame {}", i);
        }
        sleep(Duration::from_millis(33)).await;
    }

    // Check if Alice received any frames
    println!("\n9. Checking Alice's received frames...");
    let remote_users = alice_sfu.get_remote_users().await;
    println!("   Alice sees {} remote users with video", remote_users.len());

    for user_id in &remote_users {
        if let Some(frame) = alice_sfu.get_remote_frame(*user_id).await {
            println!("   ✓ Received frame from {}: {}x{}", user_id, frame.width, frame.height);
        }
    }

    if remote_users.is_empty() {
        println!("   ✗ No remote video received!");
        println!("\n   Debug: Checking Bob's remote users...");
        let bob_remote = bob_sfu.get_remote_users().await;
        println!("   Bob sees {} remote users with video", bob_remote.len());
    }

    // Cleanup
    println!("\n10. Cleaning up...");
    bob_sfu.disconnect().await?;
    alice_sfu.disconnect().await?;
    bob_client.leave_voice().await;
    alice_client.leave_voice().await;

    println!("\n=== Test Complete ===");
    Ok(())
}
