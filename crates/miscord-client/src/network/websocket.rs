use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use miscord_protocol::{ClientMessage, ServerMessage};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use crate::state::AppState;

pub struct WebSocketClient {
    sender: mpsc::Sender<ClientMessage>,
}

impl WebSocketClient {
    pub async fn connect(url: &str, token: &str, state: AppState) -> Result<Self> {
        let (ws_stream, _) = connect_async(url).await?;

        let (mut write, mut read) = ws_stream.split();

        // Create channel for sending messages
        let (tx, mut rx) = mpsc::channel::<ClientMessage>(100);

        // Authenticate
        let auth_msg = ClientMessage::Authenticate {
            token: token.to_string(),
        };
        let json = serde_json::to_string(&auth_msg)?;
        write.send(Message::Text(json.into())).await?;

        // Wait for authentication response
        if let Some(Ok(Message::Text(text))) = read.next().await {
            let response: ServerMessage = serde_json::from_str(&text)?;
            match response {
                ServerMessage::Authenticated { connection_id } => {
                    tracing::info!("WebSocket authenticated with connection ID: {}", connection_id);
                }
                ServerMessage::Error { message } => {
                    anyhow::bail!("Authentication failed: {}", message);
                }
                _ => {
                    anyhow::bail!("Unexpected response during authentication");
                }
            }
        } else {
            anyhow::bail!("Connection closed during authentication");
        }

        // Spawn task to handle outgoing messages
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::error!("Failed to serialize message: {}", e);
                        continue;
                    }
                };

                if write.send(Message::Text(json.into())).await.is_err() {
                    tracing::error!("Failed to send WebSocket message");
                    break;
                }
            }
        });

        // Spawn task to handle incoming messages
        let state_clone = state.clone();
        tokio::spawn(async move {
            while let Some(result) = read.next().await {
                match result {
                    Ok(Message::Text(text)) => {
                        if let Ok(msg) = serde_json::from_str::<ServerMessage>(&text) {
                            Self::handle_message(&state_clone, msg).await;
                        }
                    }
                    Ok(Message::Ping(data)) => {
                        // Pong is handled automatically by tungstenite
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket closed by server");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // Mark as disconnected
            let mut s = state_clone.write().await;
            s.is_connected = false;
        });

        // Mark as connected
        {
            let mut s = state.write().await;
            s.is_connected = true;
        }

        // Start ping task
        let tx_ping = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if tx_ping.send(ClientMessage::Ping).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self { sender: tx })
    }

    async fn handle_message(state: &AppState, message: ServerMessage) {
        match message {
            ServerMessage::MessageCreated { message } => {
                state.add_message(message).await;
            }
            ServerMessage::MessageUpdated { message } => {
                let mut s = state.write().await;
                if let Some(messages) = s.messages.get_mut(&message.channel_id) {
                    if let Some(pos) = messages.iter().position(|m| m.id == message.id) {
                        messages[pos] = message;
                    }
                }
            }
            ServerMessage::MessageDeleted {
                message_id,
                channel_id,
            } => {
                let mut s = state.write().await;
                if let Some(messages) = s.messages.get_mut(&channel_id) {
                    messages.retain(|m| m.id != message_id);
                }
            }
            ServerMessage::VoiceStateUpdate {
                channel_id,
                user_id,
                state: voice_state,
            } => {
                let mut s = state.write().await;
                if let Some(participant) = s.voice_participants.get_mut(&user_id) {
                    participant.is_muted = voice_state.self_muted;
                    participant.is_deafened = voice_state.self_deafened;
                    participant.is_video_enabled = voice_state.video_enabled;
                    participant.is_screen_sharing = voice_state.screen_sharing;
                }
            }
            ServerMessage::UserTyping { channel_id, user_id } => {
                // TODO: Show typing indicator
            }
            ServerMessage::UserStoppedTyping { channel_id, user_id } => {
                // TODO: Hide typing indicator
            }
            ServerMessage::ChannelSubscribed { channel_id } => {
                tracing::debug!("Subscribed to channel {}", channel_id);
            }
            ServerMessage::RtcOffer { from_user_id, sdp } => {
                // TODO: Handle WebRTC offer
                tracing::debug!("Received RTC offer from {}", from_user_id);
            }
            ServerMessage::RtcAnswer { from_user_id, sdp } => {
                // TODO: Handle WebRTC answer
                tracing::debug!("Received RTC answer from {}", from_user_id);
            }
            ServerMessage::RtcIceCandidate {
                from_user_id,
                candidate,
            } => {
                // TODO: Handle ICE candidate
                tracing::debug!("Received ICE candidate from {}", from_user_id);
            }
            ServerMessage::Pong => {
                // Heartbeat response
            }
            ServerMessage::Error { message } => {
                tracing::error!("Server error: {}", message);
            }
            _ => {}
        }
    }

    pub async fn subscribe_channel(&self, channel_id: Uuid) {
        let _ = self
            .sender
            .send(ClientMessage::SubscribeChannel { channel_id })
            .await;
    }

    pub async fn unsubscribe_channel(&self, channel_id: Uuid) {
        let _ = self
            .sender
            .send(ClientMessage::UnsubscribeChannel { channel_id })
            .await;
    }

    pub async fn start_typing(&self, channel_id: Uuid) {
        let _ = self
            .sender
            .send(ClientMessage::StartTyping { channel_id })
            .await;
    }

    pub async fn stop_typing(&self, channel_id: Uuid) {
        let _ = self
            .sender
            .send(ClientMessage::StopTyping { channel_id })
            .await;
    }

    pub async fn send_rtc_offer(&self, target_user_id: Uuid, sdp: String) {
        let _ = self
            .sender
            .send(ClientMessage::RtcOffer {
                target_user_id,
                sdp,
            })
            .await;
    }

    pub async fn send_rtc_answer(&self, target_user_id: Uuid, sdp: String) {
        let _ = self
            .sender
            .send(ClientMessage::RtcAnswer {
                target_user_id,
                sdp,
            })
            .await;
    }

    pub async fn send_ice_candidate(&self, target_user_id: Uuid, candidate: String) {
        let _ = self
            .sender
            .send(ClientMessage::RtcIceCandidate {
                target_user_id,
                candidate,
            })
            .await;
    }
}
