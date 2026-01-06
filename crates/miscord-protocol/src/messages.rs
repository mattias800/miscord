use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{MessageData, VoiceStateData};

/// Messages sent from client to server via WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Authenticate with the server
    Authenticate { token: String },

    /// Subscribe to channel updates
    SubscribeChannel { channel_id: Uuid },

    /// Unsubscribe from channel updates
    UnsubscribeChannel { channel_id: Uuid },

    /// Ping to keep connection alive
    Ping,

    /// Start typing indicator
    StartTyping { channel_id: Uuid },

    /// Stop typing indicator
    StopTyping { channel_id: Uuid },

    /// WebRTC offer
    RtcOffer { target_user_id: Uuid, sdp: String },

    /// WebRTC answer
    RtcAnswer { target_user_id: Uuid, sdp: String },

    /// WebRTC ICE candidate
    RtcIceCandidate { target_user_id: Uuid, candidate: String },

    /// SFU: Send WebRTC offer to server for video streaming
    SfuOffer { channel_id: Uuid, sdp: String },

    /// SFU: Send WebRTC answer to server (for renegotiation)
    SfuAnswer { sdp: String },

    /// SFU: Send ICE candidate to server
    SfuIceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
}

/// Messages sent from server to client via WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication successful
    Authenticated { connection_id: Uuid },

    /// Error message
    Error { message: String },

    /// Pong response to ping
    Pong,

    /// Subscribed to channel
    ChannelSubscribed { channel_id: Uuid },

    /// New message created
    MessageCreated { message: MessageData },

    /// Message updated
    MessageUpdated { message: MessageData },

    /// Message deleted
    MessageDeleted { message_id: Uuid, channel_id: Uuid },

    /// Reaction added to message
    ReactionAdded {
        message_id: Uuid,
        user_id: Uuid,
        emoji: String,
    },

    /// Reaction removed from message
    ReactionRemoved {
        message_id: Uuid,
        user_id: Uuid,
        emoji: String,
    },

    /// User started typing
    UserTyping { channel_id: Uuid, user_id: Uuid },

    /// User stopped typing
    UserStoppedTyping { channel_id: Uuid, user_id: Uuid },

    /// User's presence updated
    PresenceUpdate { user_id: Uuid, status: String },

    /// Voice state updated
    VoiceStateUpdate {
        channel_id: Uuid,
        user_id: Uuid,
        state: VoiceStateData,
    },

    /// User joined voice channel
    VoiceUserJoined { channel_id: Uuid, user_id: Uuid },

    /// User left voice channel
    VoiceUserLeft { channel_id: Uuid, user_id: Uuid },

    /// WebRTC offer from another user
    RtcOffer { from_user_id: Uuid, sdp: String },

    /// WebRTC answer from another user
    RtcAnswer { from_user_id: Uuid, sdp: String },

    /// WebRTC ICE candidate from another user
    RtcIceCandidate { from_user_id: Uuid, candidate: String },

    /// SFU: WebRTC answer from server
    SfuAnswer { sdp: String },

    /// SFU: ICE candidate from server
    SfuIceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },

    /// SFU: A new video track was added (another user started streaming)
    SfuTrackAdded {
        user_id: Uuid,
        track_id: String,
        kind: String,
    },

    /// SFU: A video track was removed (user stopped streaming)
    SfuTrackRemoved { user_id: Uuid, track_id: String },

    /// SFU: Server needs to renegotiate (new track available)
    SfuRenegotiate { sdp: String },
}
