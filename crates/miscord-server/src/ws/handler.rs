use crate::auth::verify_token;
use crate::sfu::TrackRouter;
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
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_remote::TrackRemote;

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

    tracing::info!("User {} authenticated on WebSocket", user_id);

    // Create channel for outbound messages
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Register connection with the connection manager
    state.connections.add_connection(connection_id, user_id, tx).await;

    // Update user status to online
    if let Err(e) = state
        .user_service
        .update_status(user_id, crate::models::UserStatus::Online)
        .await
    {
        tracing::error!("Failed to update user status: {}", e);
    }

    // Spawn task to forward messages from channel to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

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

                handle_client_message(&state, user_id, connection_id, client_msg).await;
            }
            Ok(Message::Ping(data)) => {
                // Send pong via connection manager
                state.connections.send_to_connection(
                    connection_id,
                    &ServerMessage::Pong,
                ).await;
                let _ = data; // Suppress warning
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

    // Abort the send task
    send_task.abort();

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

            state.connections.send_to_connection(
                connection_id,
                &ServerMessage::ChannelSubscribed { channel_id },
            ).await;
        }
        ClientMessage::UnsubscribeChannel { channel_id } => {
            state
                .connections
                .unsubscribe_from_channel(connection_id, channel_id)
                .await;
        }
        ClientMessage::Ping => {
            state.connections.send_to_connection(
                connection_id,
                &ServerMessage::Pong,
            ).await;
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
        // SFU signaling
        ClientMessage::SfuOffer { channel_id, sdp } => {
            handle_sfu_offer(state, user_id, connection_id, channel_id, sdp).await;
        }
        ClientMessage::SfuAnswer { sdp } => {
            handle_sfu_answer(state, user_id, sdp).await;
        }
        ClientMessage::SfuIceCandidate {
            candidate,
            sdp_mid,
            sdp_mline_index,
        } => {
            handle_sfu_ice_candidate(state, user_id, candidate, sdp_mid, sdp_mline_index).await;
        }
    }
}

/// Handle SFU offer from client
async fn handle_sfu_offer(
    state: &AppState,
    user_id: Uuid,
    connection_id: Uuid,
    channel_id: Uuid,
    sdp: String,
) {
    tracing::info!("Received SFU offer from user {} for channel {}", user_id, channel_id);

    // Create peer connection for this user
    let peer_connection = match state.sfu.create_peer_connection(channel_id, user_id).await {
        Ok(pc) => pc,
        Err(e) => {
            tracing::error!("Failed to create peer connection: {}", e);
            state.connections.send_to_connection(
                connection_id,
                &ServerMessage::Error {
                    message: format!("Failed to create peer connection: {}", e),
                },
            ).await;
            return;
        }
    };

    // Set up track handler for incoming video tracks
    let state_clone = state.clone();
    let channel_id_clone = channel_id;
    let user_id_clone = user_id;

    peer_connection.on_track(Box::new(move |track, _receiver, _transceiver| {
        let state = state_clone.clone();
        let channel_id = channel_id_clone;
        let user_id = user_id_clone;

        Box::pin(async move {
            handle_incoming_track(state, channel_id, user_id, track).await;
        })
    }));

    // Set up ICE candidate handler
    let state_clone = state.clone();
    let connection_id_clone = connection_id;

    peer_connection.on_ice_candidate(Box::new(move |candidate| {
        let state = state_clone.clone();
        let connection_id = connection_id_clone;

        Box::pin(async move {
            if let Some(candidate) = candidate {
                let candidate_json = match candidate.to_json() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to serialize ICE candidate: {}", e);
                        return;
                    }
                };

                state.connections.send_to_connection(
                    connection_id,
                    &ServerMessage::SfuIceCandidate {
                        candidate: candidate_json.candidate,
                        sdp_mid: candidate_json.sdp_mid,
                        sdp_mline_index: candidate_json.sdp_mline_index,
                    },
                ).await;
            }
        })
    }));

    // Parse and set remote description (the offer)
    let offer = RTCSessionDescription::offer(sdp).unwrap();
    if let Err(e) = peer_connection.set_remote_description(offer).await {
        tracing::error!("Failed to set remote description: {}", e);
        state.connections.send_to_connection(
            connection_id,
            &ServerMessage::Error {
                message: format!("Failed to set remote description: {}", e),
            },
        ).await;
        return;
    }

    // Add existing tracks from other publishers to this peer connection
    if let Some(session) = state.sfu.get_session(channel_id).await {
        let publishers = session.get_publishers().await;
        for publisher_id in publishers {
            if publisher_id == user_id {
                continue; // Don't add our own tracks
            }

            let routers = session.get_user_routers(publisher_id).await;
            for router in routers {
                // Create a local track for this subscriber
                let local_track = router.add_subscriber(user_id).await;

                // Add track to peer connection
                if let Err(e) = peer_connection.add_track(local_track).await {
                    tracing::error!("Failed to add track to peer connection: {}", e);
                }

                // Notify client about the track
                state.connections.send_to_connection(
                    connection_id,
                    &ServerMessage::SfuTrackAdded {
                        user_id: publisher_id,
                        track_id: router.track_id().to_string(),
                        kind: format!("{:?}", router.codec()),
                    },
                ).await;
            }
        }
    }

    // Create answer
    let answer = match peer_connection.create_answer(None).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to create answer: {}", e);
            state.connections.send_to_connection(
                connection_id,
                &ServerMessage::Error {
                    message: format!("Failed to create answer: {}", e),
                },
            ).await;
            return;
        }
    };

    // Set local description
    if let Err(e) = peer_connection.set_local_description(answer.clone()).await {
        tracing::error!("Failed to set local description: {}", e);
        state.connections.send_to_connection(
            connection_id,
            &ServerMessage::Error {
                message: format!("Failed to set local description: {}", e),
            },
        ).await;
        return;
    }

    // Send answer to client
    state.connections.send_to_connection(
        connection_id,
        &ServerMessage::SfuAnswer { sdp: answer.sdp },
    ).await;

    tracing::info!("Sent SFU answer to user {}", user_id);

    // If we added any tracks from existing publishers, we need to renegotiate
    // because the initial answer can only respond to what was in the offer.
    // New tracks require a new offer from the server.
    if let Some(session) = state.sfu.get_session(channel_id).await {
        let has_existing_tracks = {
            let publishers = session.get_publishers().await;
            publishers.iter().any(|&p| p != user_id)
        };

        if has_existing_tracks {
            tracing::info!("Triggering renegotiation for user {} to receive existing tracks", user_id);

            // Create renegotiation offer
            if let Ok(offer) = peer_connection.create_offer(None).await {
                if let Ok(()) = peer_connection.set_local_description(offer.clone()).await {
                    state.connections.send_to_connection(
                        connection_id,
                        &ServerMessage::SfuRenegotiate { sdp: offer.sdp },
                    ).await;
                    tracing::info!("Sent renegotiation offer to user {}", user_id);
                } else {
                    tracing::error!("Failed to set local description for renegotiation");
                }
            } else {
                tracing::error!("Failed to create renegotiation offer");
            }
        }
    }
}

/// Handle incoming track from a publisher
async fn handle_incoming_track(
    state: AppState,
    channel_id: Uuid,
    user_id: Uuid,
    track: Arc<TrackRemote>,
) {
    let track_id = track.id().to_string();
    let kind = track.kind();

    tracing::info!(
        "Received track {} ({:?}) from user {} in channel {}",
        track_id,
        kind,
        user_id,
        channel_id
    );

    // Only handle video tracks for now
    if kind != RTPCodecType::Video {
        tracing::debug!("Ignoring non-video track: {:?}", kind);
        return;
    }

    // Create track router
    let router = Arc::new(TrackRouter::new(track, user_id, track_id.clone()));

    // Add router to session
    if let Some(session) = state.sfu.get_session(channel_id).await {
        // Add as subscriber to all other users in the session
        let users = session.get_users().await;
        for other_user_id in users {
            if other_user_id == user_id {
                continue; // Don't subscribe to our own track
            }

            // Get peer connection for this user
            if let Some(pc) = session.peer_connections.read().await.get(&other_user_id) {
                let local_track = router.add_subscriber(other_user_id).await;

                // Add track to peer connection
                if let Err(e) = pc.add_track(local_track).await {
                    tracing::error!(
                        "Failed to add track to peer connection for user {}: {}",
                        other_user_id,
                        e
                    );
                    continue;
                }

                // Notify the subscriber about the new track
                state.connections.send_to_user(
                    other_user_id,
                    &ServerMessage::SfuTrackAdded {
                        user_id,
                        track_id: track_id.clone(),
                        kind: format!("{:?}", kind),
                    },
                ).await;

                // Trigger renegotiation for this user
                // The client will need to handle renegotiation
                if let Ok(offer) = pc.create_offer(None).await {
                    if pc.set_local_description(offer.clone()).await.is_ok() {
                        state.connections.send_to_user(
                            other_user_id,
                            &ServerMessage::SfuRenegotiate { sdp: offer.sdp },
                        ).await;
                    }
                }
            }
        }

        // Store the router
        session.add_track_router(user_id, router.clone()).await;
    }

    // Start forwarding loop
    let router_clone = router.clone();
    tokio::spawn(async move {
        router_clone.start_forwarding().await;
    });

    tracing::info!("Track router started for track {} from user {}", track_id, user_id);
}

/// Handle SFU answer from client (for renegotiation)
async fn handle_sfu_answer(state: &AppState, user_id: Uuid, sdp: String) {
    tracing::info!("Received SFU answer from user {}", user_id);

    // Find the user's peer connection across all sessions
    let sessions = state.sfu.sessions.read().await;
    for session in sessions.values() {
        if let Some(pc) = session.peer_connections.read().await.get(&user_id) {
            let answer = RTCSessionDescription::answer(sdp.clone()).unwrap();
            if let Err(e) = pc.set_remote_description(answer).await {
                tracing::error!("Failed to set remote description: {}", e);
            }
            return;
        }
    }

    tracing::warn!("No peer connection found for user {} to handle answer", user_id);
}

/// Handle ICE candidate from client
async fn handle_sfu_ice_candidate(
    state: &AppState,
    user_id: Uuid,
    candidate: String,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
) {
    tracing::debug!("Received ICE candidate from user {}", user_id);

    // Find the user's peer connection across all sessions
    let sessions = state.sfu.sessions.read().await;
    for session in sessions.values() {
        if let Some(pc) = session.peer_connections.read().await.get(&user_id) {
            let ice_candidate = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                candidate,
                sdp_mid,
                sdp_mline_index: sdp_mline_index.map(|i| i as u16),
                ..Default::default()
            };

            if let Err(e) = pc.add_ice_candidate(ice_candidate).await {
                tracing::error!("Failed to add ICE candidate: {}", e);
            }
            return;
        }
    }

    tracing::warn!("No peer connection found for user {} to add ICE candidate", user_id);
}
