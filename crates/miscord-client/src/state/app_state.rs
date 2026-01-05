use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use uuid::Uuid;

use miscord_protocol::{ChannelData, MessageData, ServerData, UserData};

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

#[derive(Debug)]
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

    // Typing indicators (channel_id -> (user_id -> started_at))
    pub typing_users: HashMap<Uuid, HashMap<Uuid, Instant>>,

    // Voice state
    pub voice_channel_id: Option<Uuid>,
    pub voice_participants: HashMap<Uuid, VoiceParticipant>,
    pub is_muted: bool,
    pub is_deafened: bool,
    pub is_video_enabled: bool,
    pub is_screen_sharing: bool,

    // WebRTC signaling
    pub pending_rtc_offers: Vec<RtcSignal>,
    pub pending_rtc_answers: Vec<RtcSignal>,
    pub pending_ice_candidates: Vec<IceCandidate>,

    // Connection state
    pub is_connected: bool,
    pub connection_error: Option<String>,
}

impl Default for AppStateInner {
    fn default() -> Self {
        Self {
            current_user: None,
            auth_token: None,
            servers: HashMap::new(),
            current_server_id: None,
            channels: HashMap::new(),
            current_channel_id: None,
            messages: HashMap::new(),
            users: HashMap::new(),
            typing_users: HashMap::new(),
            voice_channel_id: None,
            voice_participants: HashMap::new(),
            is_muted: false,
            is_deafened: false,
            is_video_enabled: false,
            is_screen_sharing: false,
            pending_rtc_offers: Vec::new(),
            pending_rtc_answers: Vec::new(),
            pending_ice_candidates: Vec::new(),
            is_connected: false,
            connection_error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RtcSignal {
    pub from_user_id: Uuid,
    pub sdp: String,
}

#[derive(Debug, Clone)]
pub struct IceCandidate {
    pub from_user_id: Uuid,
    pub candidate: String,
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

    // Typing indicator methods
    pub async fn set_user_typing(&self, channel_id: Uuid, user_id: Uuid) {
        let mut state = self.inner.write().await;
        state
            .typing_users
            .entry(channel_id)
            .or_default()
            .insert(user_id, Instant::now());
    }

    pub async fn clear_user_typing(&self, channel_id: Uuid, user_id: Uuid) {
        let mut state = self.inner.write().await;
        if let Some(users) = state.typing_users.get_mut(&channel_id) {
            users.remove(&user_id);
        }
    }

    pub async fn get_typing_users(&self, channel_id: Uuid) -> Vec<Uuid> {
        let state = self.inner.read().await;
        let now = Instant::now();
        let timeout = std::time::Duration::from_secs(5);

        state
            .typing_users
            .get(&channel_id)
            .map(|users| {
                users
                    .iter()
                    .filter(|(_, started_at)| now.duration_since(**started_at) < timeout)
                    .map(|(user_id, _)| *user_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    // WebRTC signaling methods
    pub async fn add_rtc_offer(&self, from_user_id: Uuid, sdp: String) {
        let mut state = self.inner.write().await;
        state.pending_rtc_offers.push(RtcSignal { from_user_id, sdp });
    }

    pub async fn add_rtc_answer(&self, from_user_id: Uuid, sdp: String) {
        let mut state = self.inner.write().await;
        state.pending_rtc_answers.push(RtcSignal { from_user_id, sdp });
    }

    pub async fn add_ice_candidate(&self, from_user_id: Uuid, candidate: String) {
        let mut state = self.inner.write().await;
        state.pending_ice_candidates.push(IceCandidate {
            from_user_id,
            candidate,
        });
    }

    pub async fn take_pending_rtc_offers(&self) -> Vec<RtcSignal> {
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_rtc_offers)
    }

    pub async fn take_pending_rtc_answers(&self) -> Vec<RtcSignal> {
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_rtc_answers)
    }

    pub async fn take_pending_ice_candidates(&self) -> Vec<IceCandidate> {
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_ice_candidates)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
