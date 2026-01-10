use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use miscord_protocol::{ClientMessage, ServerMessage, TrackType};
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
                channel_id: _,
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
            ServerMessage::VoiceUserJoined { channel_id, user_id } => {
                tracing::info!("User {} joined voice channel {}", user_id, channel_id);
                let mut s = state.write().await;
                // Only add if we're in the same channel
                if s.voice_channel_id == Some(channel_id) {
                    // Look up username from users or members
                    let username = s.users.get(&user_id)
                        .map(|u| u.username.clone())
                        .or_else(|| {
                            // Try to find in members
                            s.members.values()
                                .flatten()
                                .find(|m| m.id == user_id)
                                .map(|m| m.username.clone())
                        })
                        .unwrap_or_else(|| format!("User {}", &user_id.to_string()[..8]));

                    s.voice_participants.insert(user_id, crate::state::VoiceParticipant {
                        user_id,
                        username,
                        is_muted: false,
                        is_deafened: false,
                        is_video_enabled: false,
                        is_screen_sharing: false,
                        is_speaking: false,
                        speaking_since: None,
                    });
                }
            }
            ServerMessage::VoiceUserLeft { channel_id, user_id } => {
                tracing::info!("User {} left voice channel {}", user_id, channel_id);
                let mut s = state.write().await;
                if s.voice_channel_id == Some(channel_id) {
                    s.voice_participants.remove(&user_id);
                }
            }
            ServerMessage::UserTyping { channel_id, user_id } => {
                state.set_user_typing(channel_id, user_id).await;
                tracing::debug!("User {} is typing in channel {}", user_id, channel_id);
            }
            ServerMessage::UserStoppedTyping { channel_id, user_id } => {
                state.clear_user_typing(channel_id, user_id).await;
                tracing::debug!("User {} stopped typing in channel {}", user_id, channel_id);
            }
            ServerMessage::ReactionAdded { message_id, user_id, emoji } => {
                state.add_reaction(message_id, user_id, &emoji).await;
                tracing::debug!("Reaction {} added to message {} by user {}", emoji, message_id, user_id);
            }
            ServerMessage::ReactionRemoved { message_id, user_id, emoji } => {
                state.remove_reaction(message_id, user_id, &emoji).await;
                tracing::debug!("Reaction {} removed from message {} by user {}", emoji, message_id, user_id);
            }
            ServerMessage::ChannelSubscribed { channel_id } => {
                tracing::debug!("Subscribed to channel {}", channel_id);
            }
            ServerMessage::RtcOffer { from_user_id, sdp } => {
                tracing::debug!("Received RTC offer from {}", from_user_id);
                state.add_rtc_offer(from_user_id, sdp).await;
            }
            ServerMessage::RtcAnswer { from_user_id, sdp } => {
                tracing::debug!("Received RTC answer from {}", from_user_id);
                state.add_rtc_answer(from_user_id, sdp).await;
            }
            ServerMessage::RtcIceCandidate {
                from_user_id,
                candidate,
            } => {
                tracing::debug!("Received ICE candidate from {}", from_user_id);
                state.add_ice_candidate(from_user_id, candidate).await;
            }
            // SFU messages
            ServerMessage::SfuAnswer { sdp } => {
                tracing::info!("Received SFU answer");
                state.set_sfu_answer(sdp).await;
            }
            ServerMessage::SfuIceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => {
                tracing::debug!("Received SFU ICE candidate");
                state.add_sfu_ice_candidate(candidate, sdp_mid, sdp_mline_index).await;
            }
            ServerMessage::SfuTrackAdded {
                user_id,
                track_id,
                kind,
                track_type,
            } => {
                tracing::info!(
                    "SFU track added: user={}, track={}, kind={}, type={:?}",
                    user_id,
                    track_id,
                    kind,
                    track_type
                );
                state.sfu_track_added(user_id, track_id, kind, track_type).await;
            }
            ServerMessage::SfuTrackRemoved { user_id, track_id } => {
                tracing::info!("SFU track removed: user={}, track={}", user_id, track_id);
                state.sfu_track_removed(user_id, track_id).await;
            }
            ServerMessage::SfuRenegotiate { sdp } => {
                tracing::info!("SFU renegotiation requested");
                state.set_sfu_renegotiate(sdp).await;
            }
            ServerMessage::SfuRequestKeyframe { track_type } => {
                tracing::info!("Server requested keyframe for {:?} track", track_type);
                state.sfu_request_keyframe(track_type).await;
            }
            ServerMessage::Pong => {
                // Heartbeat response
            }
            ServerMessage::Error { message } => {
                tracing::error!("Server error: {}", message);
            }
            // Thread messages
            ServerMessage::ThreadReplyCreated {
                parent_message_id,
                message,
            } => {
                state.add_thread_reply(parent_message_id, message).await;
            }
            ServerMessage::ThreadMetadataUpdated {
                message_id,
                reply_count,
                last_reply_at,
            } => {
                state.update_thread_metadata(message_id, reply_count, last_reply_at).await;
            }
            // Pinned messages
            ServerMessage::MessagePinned {
                message_id,
                channel_id,
                pinned_by_id: _,
                pinned_by_name,
                pinned_at,
            } => {
                state.mark_message_pinned(message_id, channel_id, pinned_at, pinned_by_name).await;
            }
            ServerMessage::MessageUnpinned {
                message_id,
                channel_id,
            } => {
                state.mark_message_unpinned(message_id, channel_id).await;
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

    // SFU signaling methods
    pub async fn send_sfu_offer(&self, channel_id: Uuid, sdp: String) {
        let _ = self
            .sender
            .send(ClientMessage::SfuOffer { channel_id, sdp })
            .await;
    }

    pub async fn send_sfu_answer(&self, sdp: String) {
        let _ = self
            .sender
            .send(ClientMessage::SfuAnswer { sdp })
            .await;
    }

    pub async fn send_sfu_ice_candidate(
        &self,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    ) {
        let _ = self
            .sender
            .send(ClientMessage::SfuIceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            })
            .await;
    }

    /// Subscribe to a user's screen share track
    pub async fn subscribe_screen_track(&self, user_id: Uuid) {
        tracing::info!("Subscribing to screen share from user {}", user_id);
        let _ = self
            .sender
            .send(ClientMessage::SfuSubscribeTrack {
                user_id,
                track_type: TrackType::Screen,
            })
            .await;
    }

    /// Unsubscribe from a user's screen share track
    pub async fn unsubscribe_screen_track(&self, user_id: Uuid) {
        tracing::info!("Unsubscribing from screen share from user {}", user_id);
        let _ = self
            .sender
            .send(ClientMessage::SfuUnsubscribeTrack {
                user_id,
                track_type: TrackType::Screen,
            })
            .await;
    }

    /// Subscribe to thread updates
    pub async fn subscribe_thread(&self, parent_message_id: Uuid) {
        tracing::debug!("Subscribing to thread {}", parent_message_id);
        let _ = self
            .sender
            .send(ClientMessage::SubscribeThread { parent_message_id })
            .await;
    }

    /// Unsubscribe from thread updates
    pub async fn unsubscribe_thread(&self, parent_message_id: Uuid) {
        tracing::debug!("Unsubscribing from thread {}", parent_message_id);
        let _ = self
            .sender
            .send(ClientMessage::UnsubscribeThread { parent_message_id })
            .await;
    }
}
