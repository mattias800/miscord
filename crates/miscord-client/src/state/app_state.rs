use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use miscord_protocol::{ChannelData, MessageData, ServerData, UserData};

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

#[derive(Debug, Default)]
pub struct AppStateInner {
    // Current user
    pub current_user: Option<UserData>,
    pub auth_token: Option<String>,

    // Servers
    pub servers: HashMap<Uuid, ServerData>,
    pub current_server_id: Option<Uuid>,

    // Channels
    pub channels: HashMap<Uuid, ChannelData>,
    pub current_channel_id: Option<Uuid>,

    // Messages (channel_id -> messages)
    pub messages: HashMap<Uuid, Vec<MessageData>>,

    // Users
    pub users: HashMap<Uuid, UserData>,

    // Voice state
    pub voice_channel_id: Option<Uuid>,
    pub voice_participants: HashMap<Uuid, VoiceParticipant>,
    pub is_muted: bool,
    pub is_deafened: bool,
    pub is_video_enabled: bool,
    pub is_screen_sharing: bool,

    // Connection state
    pub is_connected: bool,
    pub connection_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VoiceParticipant {
    pub user_id: Uuid,
    pub username: String,
    pub is_muted: bool,
    pub is_deafened: bool,
    pub is_video_enabled: bool,
    pub is_screen_sharing: bool,
    pub is_speaking: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(AppStateInner::default())),
        }
    }

    pub async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, AppStateInner> {
        self.inner.read().await
    }

    pub async fn write(&self) -> tokio::sync::RwLockWriteGuard<'_, AppStateInner> {
        self.inner.write().await
    }

    pub async fn set_auth(&self, token: String, user: UserData) {
        let mut state = self.inner.write().await;
        state.auth_token = Some(token);
        state.current_user = Some(user);
    }

    pub async fn clear_auth(&self) {
        let mut state = self.inner.write().await;
        state.auth_token = None;
        state.current_user = None;
        state.servers.clear();
        state.channels.clear();
        state.messages.clear();
    }

    pub async fn is_authenticated(&self) -> bool {
        self.inner.read().await.auth_token.is_some()
    }

    pub async fn add_message(&self, message: MessageData) {
        let mut state = self.inner.write().await;
        state
            .messages
            .entry(message.channel_id)
            .or_default()
            .push(message);
    }

    pub async fn set_servers(&self, servers: Vec<ServerData>) {
        let mut state = self.inner.write().await;
        state.servers = servers.into_iter().map(|s| (s.id, s)).collect();
    }

    pub async fn set_channels(&self, channels: Vec<ChannelData>) {
        let mut state = self.inner.write().await;
        for channel in channels {
            state.channels.insert(channel.id, channel);
        }
    }

    pub async fn select_server(&self, server_id: Uuid) {
        let mut state = self.inner.write().await;
        state.current_server_id = Some(server_id);
        state.current_channel_id = None;
    }

    pub async fn select_channel(&self, channel_id: Uuid) {
        let mut state = self.inner.write().await;
        state.current_channel_id = Some(channel_id);
    }

    pub async fn join_voice(&self, channel_id: Uuid) {
        let mut state = self.inner.write().await;
        state.voice_channel_id = Some(channel_id);
        state.voice_participants.clear();
    }

    pub async fn leave_voice(&self) {
        let mut state = self.inner.write().await;
        state.voice_channel_id = None;
        state.voice_participants.clear();
        state.is_muted = false;
        state.is_deafened = false;
        state.is_video_enabled = false;
        state.is_screen_sharing = false;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
