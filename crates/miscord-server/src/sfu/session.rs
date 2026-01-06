//! SFU Session Manager
//!
//! Manages WebRTC peer connections and track routing for voice channels.

use super::TrackRouter;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use miscord_protocol::TrackType;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::api::API;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};

/// Session for a single voice channel
pub struct VoiceChannelSession {
    /// Channel ID
    pub channel_id: Uuid,
    /// Peer connections per user
    pub peer_connections: RwLock<HashMap<Uuid, Arc<RTCPeerConnection>>>,
    /// Track routers per user (for their published tracks)
    pub track_routers: RwLock<HashMap<Uuid, Vec<Arc<TrackRouter>>>>,
    /// Screen share subscriptions: screen_owner_id -> set of subscriber_ids
    /// Users must explicitly subscribe to screen shares (bandwidth optimization)
    screen_subscriptions: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
}

impl VoiceChannelSession {
    pub fn new(channel_id: Uuid) -> Self {
        Self {
            channel_id,
            peer_connections: RwLock::new(HashMap::new()),
            track_routers: RwLock::new(HashMap::new()),
            screen_subscriptions: RwLock::new(HashMap::new()),
        }
    }

    /// Get all active publishers (users with track routers)
    pub async fn get_publishers(&self) -> Vec<Uuid> {
        self.track_routers.read().await.keys().cloned().collect()
    }

    /// Get all users in the session
    pub async fn get_users(&self) -> Vec<Uuid> {
        self.peer_connections.read().await.keys().cloned().collect()
    }

    /// Get track routers for a specific user
    pub async fn get_user_routers(&self, user_id: Uuid) -> Vec<Arc<TrackRouter>> {
        self.track_routers
            .read()
            .await
            .get(&user_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Add a track router for a user
    pub async fn add_track_router(&self, user_id: Uuid, router: Arc<TrackRouter>) {
        self.track_routers
            .write()
            .await
            .entry(user_id)
            .or_default()
            .push(router);
    }

    /// Remove all track routers for a user
    pub async fn remove_user_routers(&self, user_id: Uuid) -> Vec<Arc<TrackRouter>> {
        self.track_routers
            .write()
            .await
            .remove(&user_id)
            .unwrap_or_default()
    }

    /// Get track routers for a user filtered by track type
    pub async fn get_user_routers_by_type(
        &self,
        user_id: Uuid,
        track_type: TrackType,
    ) -> Vec<Arc<TrackRouter>> {
        self.track_routers
            .read()
            .await
            .get(&user_id)
            .map(|routers| {
                routers
                    .iter()
                    .filter(|r| r.track_type() == track_type)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Subscribe a user to another user's screen share
    pub async fn subscribe_to_screen(&self, subscriber_id: Uuid, screen_owner_id: Uuid) {
        self.screen_subscriptions
            .write()
            .await
            .entry(screen_owner_id)
            .or_default()
            .insert(subscriber_id);

        tracing::info!(
            "User {} subscribed to screen share from {}",
            subscriber_id,
            screen_owner_id
        );
    }

    /// Unsubscribe a user from another user's screen share
    pub async fn unsubscribe_from_screen(&self, subscriber_id: Uuid, screen_owner_id: Uuid) {
        if let Some(subscribers) = self.screen_subscriptions.write().await.get_mut(&screen_owner_id)
        {
            subscribers.remove(&subscriber_id);
            tracing::info!(
                "User {} unsubscribed from screen share from {}",
                subscriber_id,
                screen_owner_id
            );
        }
    }

    /// Check if a user is subscribed to another user's screen share
    pub async fn is_subscribed_to_screen(&self, subscriber_id: Uuid, screen_owner_id: Uuid) -> bool {
        self.screen_subscriptions
            .read()
            .await
            .get(&screen_owner_id)
            .map(|subs| subs.contains(&subscriber_id))
            .unwrap_or(false)
    }

    /// Get all users subscribed to a screen share
    pub async fn get_screen_subscribers(&self, screen_owner_id: Uuid) -> HashSet<Uuid> {
        self.screen_subscriptions
            .read()
            .await
            .get(&screen_owner_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Remove all screen subscriptions for a user (when they leave)
    pub async fn remove_user_screen_subscriptions(&self, user_id: Uuid) {
        // Remove as screen owner
        self.screen_subscriptions.write().await.remove(&user_id);

        // Remove as subscriber from all screen shares
        for subscribers in self.screen_subscriptions.write().await.values_mut() {
            subscribers.remove(&user_id);
        }
    }
}

/// Global SFU session manager
pub struct SfuSessionManager {
    /// WebRTC API (shared for all connections)
    api: Arc<API>,
    /// Sessions per voice channel
    pub sessions: RwLock<HashMap<Uuid, Arc<VoiceChannelSession>>>,
    /// ICE servers configuration
    ice_servers: Vec<RTCIceServer>,
}

impl SfuSessionManager {
    /// Create a new SFU session manager
    pub fn new(stun_servers: Vec<String>, turn_servers: Vec<(String, String, String)>) -> Result<Self> {
        // Create media engine with H.264 codec support (hardware accelerated on clients)
        let mut media_engine = MediaEngine::default();

        // Register H.264 codec for video
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: "video/H264".to_string(),
                    clock_rate: 90000,
                    channels: 0,
                    // Profile Level ID: Baseline profile, level 3.1 (720p30)
                    // packetization-mode=1 enables NAL unit mode for efficient RTP
                    sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_string(),
                    rtcp_feedback: vec![],
                },
                payload_type: 96,
                ..Default::default()
            },
            RTPCodecType::Video,
        )?;

        // Register Opus codec for audio (in case we want audio too)
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: "audio/opus".to_string(),
                    clock_rate: 48000,
                    channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                    rtcp_feedback: vec![],
                },
                payload_type: 111,
                ..Default::default()
            },
            RTPCodecType::Audio,
        )?;

        // Create interceptor registry
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)?;

        // Create setting engine
        let setting_engine = SettingEngine::default();

        // Build API
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        // Build ICE servers list
        let mut ice_servers = vec![];

        for stun_url in stun_servers {
            ice_servers.push(RTCIceServer {
                urls: vec![stun_url],
                ..Default::default()
            });
        }

        for (url, username, credential) in turn_servers {
            ice_servers.push(RTCIceServer {
                urls: vec![url],
                username,
                credential,
                ..Default::default()
            });
        }

        Ok(Self {
            api: Arc::new(api),
            sessions: RwLock::new(HashMap::new()),
            ice_servers,
        })
    }

    /// Get or create a session for a voice channel
    pub async fn get_or_create_session(&self, channel_id: Uuid) -> Arc<VoiceChannelSession> {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get(&channel_id) {
            return session.clone();
        }

        let session = Arc::new(VoiceChannelSession::new(channel_id));
        sessions.insert(channel_id, session.clone());
        tracing::info!("Created new SFU session for channel {}", channel_id);

        session
    }

    /// Create a peer connection for a user in a channel
    pub async fn create_peer_connection(
        &self,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Result<Arc<RTCPeerConnection>> {
        let config = RTCConfiguration {
            ice_servers: self.ice_servers.clone(),
            ..Default::default()
        };

        let peer_connection = Arc::new(self.api.new_peer_connection(config).await?);

        // Store in session
        let session = self.get_or_create_session(channel_id).await;
        session
            .peer_connections
            .write()
            .await
            .insert(user_id, peer_connection.clone());

        tracing::info!(
            "Created peer connection for user {} in channel {}",
            user_id,
            channel_id
        );

        Ok(peer_connection)
    }

    /// Get peer connection for a user in a channel
    pub async fn get_peer_connection(
        &self,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Option<Arc<RTCPeerConnection>> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&channel_id)?;
        session
            .peer_connections
            .read()
            .await
            .get(&user_id)
            .cloned()
    }

    /// Remove a user from a channel session
    pub async fn remove_user(&self, channel_id: Uuid, user_id: Uuid) {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(&channel_id) {
            // Remove peer connection
            if let Some(pc) = session.peer_connections.write().await.remove(&user_id) {
                if let Err(e) = pc.close().await {
                    tracing::warn!("Error closing peer connection: {}", e);
                }
            }

            // Stop and remove track routers
            let routers = session.remove_user_routers(user_id).await;
            for router in routers {
                router.stop().await;
            }

            // Remove user as subscriber from all other routers
            let all_routers = session.track_routers.read().await;
            for routers in all_routers.values() {
                for router in routers {
                    router.remove_subscriber(user_id).await;
                }
            }

            // Clean up screen share subscriptions
            session.remove_user_screen_subscriptions(user_id).await;

            tracing::info!("Removed user {} from SFU session {}", user_id, channel_id);
        }
    }

    /// Get session for a channel
    pub async fn get_session(&self, channel_id: Uuid) -> Option<Arc<VoiceChannelSession>> {
        self.sessions.read().await.get(&channel_id).cloned()
    }

    /// Remove a session (when channel is empty)
    pub async fn remove_session(&self, channel_id: Uuid) {
        self.sessions.write().await.remove(&channel_id);
        tracing::info!("Removed SFU session for channel {}", channel_id);
    }
}
