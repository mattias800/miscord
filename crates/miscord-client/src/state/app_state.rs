use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use uuid::Uuid;

use miscord_protocol::{ChannelData, CommunityData, MessageData, UserData};

/// Tracks reaction state for a single emoji on a message.
/// Contains the set of user IDs who reacted with this emoji.
#[derive(Debug, Clone, Default)]
pub struct ReactionState {
    /// Users who reacted with this emoji
    pub user_ids: HashSet<Uuid>,
}

impl ReactionState {
    /// Total number of users who reacted
    pub fn count(&self) -> usize {
        self.user_ids.len()
    }

    /// Check if a specific user has reacted
    pub fn has_user(&self, user_id: Uuid) -> bool {
        self.user_ids.contains(&user_id)
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

#[derive(Debug)]
pub struct AppStateInner {
    // Current user
    pub current_user: Option<UserData>,
    pub auth_token: Option<String>,

    // Communities
    pub communities: HashMap<Uuid, CommunityData>,
    pub current_community_id: Option<Uuid>,

    // Channels
    pub channels: HashMap<Uuid, ChannelData>,
    pub current_channel_id: Option<Uuid>,

    // Messages (channel_id -> messages)
    pub messages: HashMap<Uuid, Vec<MessageData>>,

    // Message reactions (message_id -> emoji -> reaction state)
    pub message_reactions: HashMap<Uuid, HashMap<String, ReactionState>>,

    // Users
    pub users: HashMap<Uuid, UserData>,

    // Community members (community_id -> list of members)
    pub members: HashMap<Uuid, Vec<UserData>>,

    // Typing indicators (channel_id -> (user_id -> started_at))
    pub typing_users: HashMap<Uuid, HashMap<Uuid, Instant>>,

    // Voice state
    pub voice_channel_id: Option<Uuid>,
    pub voice_participants: HashMap<Uuid, VoiceParticipant>,
    pub voice_channel_participants: HashMap<Uuid, Vec<VoiceParticipant>>, // All voice channel participants for sidebar
    pub is_muted: bool,
    pub is_deafened: bool,
    pub is_video_enabled: bool,
    pub is_screen_sharing: bool,
    pub wants_screen_share: bool, // Flag to open screen picker (before actual sharing starts)
    pub is_speaking: bool, // Local user speaking status

    // WebRTC signaling
    pub pending_rtc_offers: Vec<RtcSignal>,
    pub pending_rtc_answers: Vec<RtcSignal>,
    pub pending_ice_candidates: Vec<IceCandidate>,

    // Connection state
    pub is_connected: bool,
    pub connection_error: Option<String>,

    // Audio settings
    pub selected_input_device: Option<String>,
    pub selected_output_device: Option<String>,
    pub input_gain_db: f32,    // -20 to +20 dB
    pub output_volume: f32,
    pub loopback_enabled: bool,
    pub gate_threshold_db: f32, // -60 to 0 dB, audio below this is muted
    pub gate_enabled: bool,

    // UI state for pending invite code
    pub pending_invite_code: Option<String>,

    // Video settings
    pub selected_video_device: Option<u32>, // Device index

    // SFU state
    pub sfu_answer: Option<String>,
    pub sfu_renegotiate: Option<String>,
    pub pending_sfu_ice_candidates: Vec<SfuIceCandidate>,
    pub sfu_tracks: HashMap<Uuid, Vec<SfuTrackInfo>>, // user_id -> tracks
    pub pending_keyframe_requests: Vec<miscord_protocol::TrackType>,

    // Thread state
    pub open_thread: Option<Uuid>, // Parent message ID of currently open thread
    pub thread_messages: HashMap<Uuid, Vec<MessageData>>, // parent_message_id -> thread replies
}

impl Default for AppStateInner {
    fn default() -> Self {
        Self {
            current_user: None,
            auth_token: None,
            communities: HashMap::new(),
            current_community_id: None,
            channels: HashMap::new(),
            current_channel_id: None,
            messages: HashMap::new(),
            message_reactions: HashMap::new(),
            users: HashMap::new(),
            members: HashMap::new(),
            typing_users: HashMap::new(),
            voice_channel_id: None,
            voice_participants: HashMap::new(),
            voice_channel_participants: HashMap::new(),
            is_muted: false,
            is_deafened: false,
            is_video_enabled: false,
            is_screen_sharing: false,
            wants_screen_share: false,
            is_speaking: false,
            pending_rtc_offers: Vec::new(),
            pending_rtc_answers: Vec::new(),
            pending_ice_candidates: Vec::new(),
            is_connected: false,
            connection_error: None,
            selected_input_device: None,
            selected_output_device: None,
            input_gain_db: 0.0,      // 0 dB = unity gain
            output_volume: 1.0,
            loopback_enabled: false,
            gate_threshold_db: -40.0, // -40 dB default threshold
            gate_enabled: true,
            pending_invite_code: None,
            selected_video_device: None,
            sfu_answer: None,
            sfu_renegotiate: None,
            pending_sfu_ice_candidates: Vec::new(),
            sfu_tracks: HashMap::new(),
            pending_keyframe_requests: Vec::new(),
            open_thread: None,
            thread_messages: HashMap::new(),
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
pub struct SfuIceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct SfuTrackInfo {
    pub track_id: String,
    pub kind: String,
    pub track_type: miscord_protocol::TrackType,
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
    pub speaking_since: Option<Instant>,
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
        state.communities.clear();
        state.channels.clear();
        state.messages.clear();
    }

    pub async fn is_authenticated(&self) -> bool {
        self.inner.read().await.auth_token.is_some()
    }

    pub async fn add_message(&self, message: MessageData) {
        let mut state = self.inner.write().await;
        // Messages are stored in DESC order (newest first)
        // Insert at front so newest message is at index 0
        state
            .messages
            .entry(message.channel_id)
            .or_default()
            .insert(0, message);
    }

    pub async fn set_communities(&self, communities: Vec<CommunityData>) {
        let mut state = self.inner.write().await;
        state.communities = communities.into_iter().map(|c| (c.id, c)).collect();
    }

    pub async fn set_channels(&self, channels: Vec<ChannelData>) {
        let mut state = self.inner.write().await;
        for channel in channels {
            state.channels.insert(channel.id, channel);
        }
    }

    pub async fn select_community(&self, community_id: Uuid) {
        let mut state = self.inner.write().await;
        state.current_community_id = Some(community_id);
        state.current_channel_id = None;
    }

    pub async fn set_members(&self, community_id: Uuid, members: Vec<UserData>) {
        let mut state = self.inner.write().await;
        state.members.insert(community_id, members);
    }

    pub async fn select_channel(&self, channel_id: Uuid) {
        let mut state = self.inner.write().await;
        state.current_channel_id = Some(channel_id);
    }

    pub async fn join_voice(&self, channel_id: Uuid) {
        let mut state = self.inner.write().await;
        state.voice_channel_id = Some(channel_id);
        state.voice_participants.clear();

        // Add self to voice participants
        if let Some(user) = state.current_user.clone() {
            let participant = VoiceParticipant {
                user_id: user.id,
                username: user.username.clone(),
                is_muted: state.is_muted,
                is_deafened: state.is_deafened,
                is_video_enabled: state.is_video_enabled,
                is_screen_sharing: state.is_screen_sharing,
                is_speaking: false,
                speaking_since: None,
            };
            state.voice_participants.insert(user.id, participant);
        }
    }

    pub async fn leave_voice(&self) {
        let mut state = self.inner.write().await;
        state.voice_channel_id = None;
        state.voice_participants.clear();
        state.is_muted = false;
        state.is_deafened = false;
        state.is_video_enabled = false;
        state.is_screen_sharing = false;
        state.wants_screen_share = false;
        state.is_speaking = false;
    }

    /// Set local user speaking status
    pub async fn set_local_speaking(&self, speaking: bool) {
        let mut state = self.inner.write().await;
        state.is_speaking = speaking;
    }

    /// Update a participant's speaking status
    pub async fn update_participant_speaking(&self, user_id: Uuid, speaking: bool) {
        let mut state = self.inner.write().await;
        if let Some(participant) = state.voice_participants.get_mut(&user_id) {
            participant.is_speaking = speaking;
            participant.speaking_since = if speaking { Some(Instant::now()) } else { None };
        }
    }

    /// Get participants for a specific voice channel (for sidebar display)
    pub async fn get_voice_channel_participants(&self, channel_id: Uuid) -> Vec<VoiceParticipant> {
        let state = self.inner.read().await;
        // If we're in this channel, return our participants
        if state.voice_channel_id == Some(channel_id) {
            state.voice_participants.values().cloned().collect()
        } else {
            // Otherwise return cached participants for other channels
            state
                .voice_channel_participants
                .get(&channel_id)
                .cloned()
                .unwrap_or_default()
        }
    }

    /// Update voice channel participants (from server broadcast)
    pub async fn set_voice_channel_participants(&self, channel_id: Uuid, participants: Vec<VoiceParticipant>) {
        let mut state = self.inner.write().await;
        state.voice_channel_participants.insert(channel_id, participants);
    }

    /// Add a participant to voice channel
    pub async fn add_voice_participant(&self, participant: VoiceParticipant) {
        let mut state = self.inner.write().await;
        state.voice_participants.insert(participant.user_id, participant);
    }

    /// Remove a participant from voice channel
    pub async fn remove_voice_participant(&self, user_id: Uuid) {
        let mut state = self.inner.write().await;
        state.voice_participants.remove(&user_id);
    }

    // Reaction methods
    pub async fn add_reaction(&self, message_id: Uuid, user_id: Uuid, emoji: &str) {
        let mut state = self.inner.write().await;
        let reaction_state = state
            .message_reactions
            .entry(message_id)
            .or_default()
            .entry(emoji.to_string())
            .or_default();

        // Add user to the set (HashSet handles duplicates automatically)
        reaction_state.user_ids.insert(user_id);
    }

    pub async fn remove_reaction(&self, message_id: Uuid, user_id: Uuid, emoji: &str) {
        let mut state = self.inner.write().await;
        if let Some(emoji_reactions) = state.message_reactions.get_mut(&message_id) {
            if let Some(reaction_state) = emoji_reactions.get_mut(emoji) {
                reaction_state.user_ids.remove(&user_id);

                // Clean up if no reactions left
                if reaction_state.count() == 0 {
                    emoji_reactions.remove(emoji);
                }
            }
            if emoji_reactions.is_empty() {
                state.message_reactions.remove(&message_id);
            }
        }
    }

    pub async fn get_reactions(&self, message_id: Uuid) -> HashMap<String, ReactionState> {
        let state = self.inner.read().await;
        state
            .message_reactions
            .get(&message_id)
            .cloned()
            .unwrap_or_default()
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

    // SFU methods
    pub async fn set_sfu_answer(&self, sdp: String) {
        let mut state = self.inner.write().await;
        state.sfu_answer = Some(sdp);
    }

    pub async fn take_sfu_answer(&self) -> Option<String> {
        let mut state = self.inner.write().await;
        state.sfu_answer.take()
    }

    pub async fn set_sfu_renegotiate(&self, sdp: String) {
        let mut state = self.inner.write().await;
        state.sfu_renegotiate = Some(sdp);
    }

    pub async fn take_sfu_renegotiate(&self) -> Option<String> {
        let mut state = self.inner.write().await;
        state.sfu_renegotiate.take()
    }

    pub async fn add_sfu_ice_candidate(
        &self,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    ) {
        let mut state = self.inner.write().await;
        state.pending_sfu_ice_candidates.push(SfuIceCandidate {
            candidate,
            sdp_mid,
            sdp_mline_index,
        });
    }

    pub async fn take_sfu_ice_candidates(&self) -> Vec<SfuIceCandidate> {
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_sfu_ice_candidates)
    }

    pub async fn sfu_track_added(&self, user_id: Uuid, track_id: String, kind: String, track_type: miscord_protocol::TrackType) {
        let mut state = self.inner.write().await;
        state
            .sfu_tracks
            .entry(user_id)
            .or_default()
            .push(SfuTrackInfo { track_id, kind, track_type });
    }

    pub async fn sfu_track_removed(&self, user_id: Uuid, track_id: String) {
        let mut state = self.inner.write().await;
        if let Some(tracks) = state.sfu_tracks.get_mut(&user_id) {
            tracks.retain(|t| t.track_id != track_id);
            if tracks.is_empty() {
                state.sfu_tracks.remove(&user_id);
            }
        }
    }

    pub async fn clear_sfu_state(&self) {
        let mut state = self.inner.write().await;
        state.sfu_answer = None;
        state.sfu_renegotiate = None;
        state.pending_sfu_ice_candidates.clear();
        state.sfu_tracks.clear();
        state.pending_keyframe_requests.clear();
    }

    /// Called when server requests a keyframe for a track type
    pub async fn sfu_request_keyframe(&self, track_type: miscord_protocol::TrackType) {
        let mut state = self.inner.write().await;
        state.pending_keyframe_requests.push(track_type);
    }

    /// Take any pending keyframe requests
    pub async fn take_pending_keyframe_requests(&self) -> Vec<miscord_protocol::TrackType> {
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_keyframe_requests)
    }

    // Thread methods

    /// Open a thread panel for a message
    pub async fn open_thread(&self, parent_message_id: Uuid) {
        let mut state = self.inner.write().await;
        state.open_thread = Some(parent_message_id);
    }

    /// Close the thread panel
    pub async fn close_thread(&self) {
        let mut state = self.inner.write().await;
        state.open_thread = None;
    }

    /// Set thread messages (when loading a thread)
    /// Also extracts reactions from messages into message_reactions state
    pub async fn set_thread_messages(&self, parent_message_id: Uuid, messages: Vec<MessageData>) {
        let mut state = self.inner.write().await;

        // Extract reactions from messages into message_reactions state
        for msg in &messages {
            if !msg.reactions.is_empty() {
                let mut emoji_reactions: HashMap<String, ReactionState> = HashMap::new();
                for reaction in &msg.reactions {
                    let reaction_state = ReactionState {
                        user_ids: reaction.user_ids.iter().copied().collect(),
                    };
                    emoji_reactions.insert(reaction.emoji.clone(), reaction_state);
                }
                state.message_reactions.insert(msg.id, emoji_reactions);
            }
        }

        state.thread_messages.insert(parent_message_id, messages);
    }

    /// Add a new thread reply (from WebSocket)
    /// Also extracts reactions from the message into message_reactions state
    pub async fn add_thread_reply(&self, parent_message_id: Uuid, message: MessageData) {
        let mut state = self.inner.write().await;

        // Extract reactions from the new message into message_reactions state
        if !message.reactions.is_empty() {
            let mut emoji_reactions: HashMap<String, ReactionState> = HashMap::new();
            for reaction in &message.reactions {
                let reaction_state = ReactionState {
                    user_ids: reaction.user_ids.iter().copied().collect(),
                };
                emoji_reactions.insert(reaction.emoji.clone(), reaction_state);
            }
            state.message_reactions.insert(message.id, emoji_reactions);
        }

        state
            .thread_messages
            .entry(parent_message_id)
            .or_default()
            .push(message);
    }

    /// Update thread metadata on a message (reply_count, last_reply_at)
    pub async fn update_thread_metadata(
        &self,
        message_id: Uuid,
        reply_count: i32,
        last_reply_at: Option<DateTime<Utc>>,
    ) {
        let mut state = self.inner.write().await;
        // Update the message in all channel message lists
        for messages in state.messages.values_mut() {
            if let Some(msg) = messages.iter_mut().find(|m| m.id == message_id) {
                msg.reply_count = reply_count;
                msg.last_reply_at = last_reply_at;
                break;
            }
        }
    }

    /// Get the currently open thread's parent message ID
    pub async fn get_open_thread(&self) -> Option<Uuid> {
        self.inner.read().await.open_thread
    }

    /// Get thread messages for a parent message
    pub async fn get_thread_messages(&self, parent_message_id: Uuid) -> Vec<MessageData> {
        self.inner
            .read()
            .await
            .thread_messages
            .get(&parent_message_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Clear thread messages (when closing thread or switching channels)
    pub async fn clear_thread_messages(&self, parent_message_id: Uuid) {
        let mut state = self.inner.write().await;
        state.thread_messages.remove(&parent_message_id);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
