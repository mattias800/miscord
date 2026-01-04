use crate::auth::verify_token;
use crate::state::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use miscord_protocol::{ClientMessage, ServerMessage};
use uuid::Uuid;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // First message should be authentication
    let auth_msg = match receiver.next().await {
        Some(Ok(Message::Text(text))) => text,
        _ => {
            tracing::warn!("WebSocket closed before authentication");
            return;
        }
    };

    // Parse auth message
    let auth: ClientMessage = match serde_json::from_str(&auth_msg) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::warn!("Invalid auth message: {}", e);
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "Invalid message format".to_string(),
                    })
                    .unwrap().into(),
                ))
                .await;
            return;
        }
    };

    let (user_id, _username) = match auth {
        ClientMessage::Authenticate { token } => {
            match verify_token(&token, &state.config.jwt_secret) {
                Ok(claims) => (claims.sub, claims.username),
                Err(_) => {
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&ServerMessage::Error {
                                message: "Invalid token".to_string(),
                            })
                            .unwrap().into(),
                        ))
                        .await;
                    return;
                }
            }
        }
        _ => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "First message must be authentication".to_string(),
                    })
                    .unwrap().into(),
                ))
                .await;
            return;
        }
    };

    // Send authentication success
    let connection_id = Uuid::new_v4();
    if sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Authenticated { connection_id })
                .unwrap().into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // Register connection
    // Note: This is a simplified version - in production you'd need to handle
    // the split sender/receiver with the connection manager differently
    // For now, we'll handle messages in a loop and broadcast via a separate mechanism

    tracing::info!("User {} authenticated on WebSocket", user_id);

    // Update user status to online
    if let Err(e) = state
        .user_service
        .update_status(user_id, crate::models::UserStatus::Online)
        .await
    {
        tracing::error!("Failed to update user status: {}", e);
    }

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let client_msg: ClientMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("Invalid message from {}: {}", user_id, e);
                        continue;
                    }
                };

                handle_client_message(&state, user_id, connection_id, client_msg, &mut sender).await;
            }
            Ok(Message::Ping(data)) => {
                if sender.send(Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Err(e) => {
                tracing::error!("WebSocket error for user {}: {}", user_id, e);
                break;
            }
            _ => {}
        }
    }

    // Cleanup on disconnect
    state.connections.remove_connection(connection_id).await;

    // Update user status to offline (if no other connections)
    if !state.connections.is_user_online(user_id).await {
        if let Err(e) = state
            .user_service
            .update_status(user_id, crate::models::UserStatus::Offline)
            .await
        {
            tracing::error!("Failed to update user status: {}", e);
        }
    }

    // Leave any voice channels
    if let Err(e) = state.channel_service.leave_voice(user_id).await {
        tracing::error!("Failed to leave voice channel: {}", e);
    }

    tracing::info!("User {} disconnected from WebSocket", user_id);
}

async fn handle_client_message(
    state: &AppState,
    user_id: Uuid,
    connection_id: Uuid,
    message: ClientMessage,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) {
    match message {
        ClientMessage::Authenticate { .. } => {
            // Already authenticated
        }
        ClientMessage::SubscribeChannel { channel_id } => {
            state
                .connections
                .subscribe_to_channel(connection_id, channel_id)
                .await;

            let response = ServerMessage::ChannelSubscribed { channel_id };
            let _ = sender
                .send(Message::Text(serde_json::to_string(&response).unwrap().into()))
                .await;
        }
        ClientMessage::UnsubscribeChannel { channel_id } => {
            state
                .connections
                .unsubscribe_from_channel(connection_id, channel_id)
                .await;
        }
        ClientMessage::Ping => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Pong).unwrap().into(),
                ))
                .await;
        }
        ClientMessage::StartTyping { channel_id } => {
            state
                .connections
                .broadcast_to_channel(
                    channel_id,
                    &ServerMessage::UserTyping {
                        channel_id,
                        user_id,
                    },
                )
                .await;
        }
        ClientMessage::StopTyping { channel_id } => {
            state
                .connections
                .broadcast_to_channel(
                    channel_id,
                    &ServerMessage::UserStoppedTyping {
                        channel_id,
                        user_id,
                    },
                )
                .await;
        }
        // WebRTC signaling
        ClientMessage::RtcOffer {
            target_user_id,
            sdp,
        } => {
            state
                .connections
                .send_to_user(
                    target_user_id,
                    &ServerMessage::RtcOffer {
                        from_user_id: user_id,
                        sdp,
                    },
                )
                .await;
        }
        ClientMessage::RtcAnswer {
            target_user_id,
            sdp,
        } => {
            state
                .connections
                .send_to_user(
                    target_user_id,
                    &ServerMessage::RtcAnswer {
                        from_user_id: user_id,
                        sdp,
                    },
                )
                .await;
        }
        ClientMessage::RtcIceCandidate {
            target_user_id,
            candidate,
        } => {
            state
                .connections
                .send_to_user(
                    target_user_id,
                    &ServerMessage::RtcIceCandidate {
                        from_user_id: user_id,
                        candidate,
                    },
                )
                .await;
        }
    }
}
