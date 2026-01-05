mod api;
mod websocket;

use crate::state::{AppState, LoginRequest, LoginResponse, RegisterRequest, RegisterResponse};
use anyhow::Result;
use miscord_protocol::{ChannelData, ChannelType, CommunityData, MessageData, UserData};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
pub struct NetworkClient {
    state: AppState,
    server_url: Arc<RwLock<String>>,
    ws_client: Arc<RwLock<Option<websocket::WebSocketClient>>>,
}

impl NetworkClient {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            server_url: Arc::new(RwLock::new(String::new())),
            ws_client: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn set_server_url(&self, url: &str) {
        *self.server_url.write().await = url.to_string();
    }

    async fn get_server_url(&self) -> String {
        self.server_url.read().await.clone()
    }

    async fn get_token(&self) -> Option<String> {
        self.state.read().await.auth_token.clone()
    }

    // Auth

    pub async fn login(&self, server_url: &str, request: LoginRequest) -> Result<(String, UserData)> {
        self.set_server_url(server_url).await;

        let response: LoginResponse = api::post(&format!("{}/api/auth/login", server_url), &request, None).await?;

        // Get user info
        let user: UserData = api::get(
            &format!("{}/api/users/me", server_url),
            Some(&response.token),
        )
        .await?;

        Ok((response.token, user))
    }

    pub async fn register(&self, server_url: &str, request: RegisterRequest) -> Result<RegisterResponse> {
        self.set_server_url(server_url).await;
        api::post(&format!("{}/api/auth/register", server_url), &request, None).await
    }

    // WebSocket

    pub async fn connect(&self) -> Result<()> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await.ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        // Convert http to ws
        let ws_url = server_url.replace("http://", "ws://").replace("https://", "wss://");
        let ws_url = format!("{}/ws", ws_url);

        let client = websocket::WebSocketClient::connect(&ws_url, &token, self.state.clone()).await?;
        *self.ws_client.write().await = Some(client);

        // Load initial data
        self.load_communities().await?;

        Ok(())
    }

    pub async fn subscribe_channel(&self, channel_id: Uuid) {
        if let Some(client) = self.ws_client.read().await.as_ref() {
            client.subscribe_channel(channel_id).await;
        }
    }

    // Communities

    pub async fn load_communities(&self) -> Result<()> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        let communities: Vec<CommunityData> = api::get(&format!("{}/api/communities", server_url), token.as_deref()).await?;
        self.state.set_communities(communities).await;

        Ok(())
    }

    pub async fn get_communities(&self) -> Result<Vec<CommunityData>> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;
        api::get(&format!("{}/api/communities", server_url), token.as_deref()).await
    }

    pub async fn create_community(&self, name: &str) -> Result<CommunityData> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        #[derive(serde::Serialize)]
        struct CreateCommunity {
            name: String,
        }

        api::post(
            &format!("{}/api/communities", server_url),
            &CreateCommunity { name: name.to_string() },
            token.as_deref(),
        )
        .await
    }

    pub async fn join_community(&self, invite_code: &str) -> Result<CommunityData> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        api::post::<CommunityData, _>(
            &format!("{}/api/invites/{}", server_url, invite_code),
            &(),
            token.as_deref(),
        )
        .await
    }

    pub async fn create_invite(&self, community_id: Uuid) -> Result<String> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        #[derive(serde::Deserialize)]
        struct InviteResponse {
            code: String,
        }

        let response: InviteResponse = api::post(
            &format!("{}/api/communities/{}/invites", server_url, community_id),
            &(),
            token.as_deref(),
        )
        .await?;

        Ok(response.code)
    }

    // Channels

    pub async fn get_channels(&self, community_id: Uuid) -> Result<Vec<ChannelData>> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;
        api::get(
            &format!("{}/api/communities/{}/channels", server_url, community_id),
            token.as_deref(),
        )
        .await
    }

    pub async fn create_channel(
        &self,
        community_id: Uuid,
        name: &str,
        channel_type: ChannelType,
    ) -> Result<ChannelData> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        #[derive(serde::Serialize)]
        struct CreateChannel {
            name: String,
            channel_type: ChannelType,
        }

        api::post(
            &format!("{}/api/communities/{}/channels", server_url, community_id),
            &CreateChannel {
                name: name.to_string(),
                channel_type,
            },
            token.as_deref(),
        )
        .await
    }

    // Members

    pub async fn get_members(&self, community_id: Uuid) -> Result<Vec<UserData>> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;
        api::get(
            &format!("{}/api/communities/{}/members", server_url, community_id),
            token.as_deref(),
        )
        .await
    }

    // Messages

    pub async fn get_messages(&self, channel_id: Uuid, before: Option<Uuid>) -> Result<Vec<MessageData>> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        let url = if let Some(before_id) = before {
            format!(
                "{}/api/channels/{}/messages?before={}",
                server_url, channel_id, before_id
            )
        } else {
            format!("{}/api/channels/{}/messages", server_url, channel_id)
        };

        api::get(&url, token.as_deref()).await
    }

    pub async fn send_message(&self, channel_id: Uuid, content: &str) -> Result<MessageData> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        #[derive(serde::Serialize)]
        struct CreateMessage {
            content: String,
        }

        api::post(
            &format!("{}/api/channels/{}/messages", server_url, channel_id),
            &CreateMessage {
                content: content.to_string(),
            },
            token.as_deref(),
        )
        .await
    }

    // Voice

    pub async fn join_voice(&self, channel_id: Uuid) -> Result<()> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        api::post_empty_void(
            &format!("{}/api/channels/{}/voice/join", server_url, channel_id),
            token.as_deref(),
        )
        .await
    }

    pub async fn leave_voice(&self) {
        let server_url = match self.server_url.read().await.as_str() {
            "" => return,
            s => s.to_string(),
        };

        let token = self.get_token().await;

        let _ = api::post_empty_void(
            &format!("{}/api/voice/leave", server_url),
            token.as_deref(),
        )
        .await;
    }

    pub async fn update_voice_state(
        &self,
        self_muted: Option<bool>,
        self_deafened: Option<bool>,
        video_enabled: Option<bool>,
        screen_sharing: Option<bool>,
    ) -> Result<()> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        #[derive(serde::Serialize)]
        struct UpdateVoiceState {
            #[serde(skip_serializing_if = "Option::is_none")]
            self_muted: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            self_deafened: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            video_enabled: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            screen_sharing: Option<bool>,
        }

        api::patch_empty(
            &format!("{}/api/voice/state", server_url),
            &UpdateVoiceState {
                self_muted,
                self_deafened,
                video_enabled,
                screen_sharing,
            },
            token.as_deref(),
        )
        .await
    }
}
