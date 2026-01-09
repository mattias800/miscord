use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// User data shared between client and server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserData {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub status: UserStatus,
    pub custom_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    #[default]
    Offline,
    Online,
    Idle,
    DoNotDisturb,
    Invisible,
}

/// Community data (equivalent to Discord's "server" or "guild")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityData {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Channel data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelData {
    pub id: Uuid,
    pub community_id: Option<Uuid>,
    pub name: String,
    pub topic: Option<String>,
    pub channel_type: ChannelType,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Text,
    Voice,
    DirectMessage,
    GroupDm,
}

/// Message data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub author_name: String,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,
    #[serde(default)]
    pub reactions: Vec<ReactionData>,
    pub created_at: DateTime<Utc>,
}

/// Voice state data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceStateData {
    pub muted: bool,
    pub deafened: bool,
    pub self_muted: bool,
    pub self_deafened: bool,
    pub video_enabled: bool,
    pub screen_sharing: bool,
}

/// ICE server configuration for WebRTC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Attachment data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentData {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
}

/// Reaction count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionData {
    pub emoji: String,
    pub count: i64,
    pub reacted_by_me: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This module simulates the server's types WITH the fix applied
    /// (serde rename_all = "snake_case")
    mod server_types {
        use serde::{Deserialize, Serialize};
        use chrono::{DateTime, Utc};
        use uuid::Uuid;

        /// Server's ChannelType - WITH #[serde(rename_all = "snake_case")]
        /// This matches the fix we applied to the server
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum ChannelType {
            Text,
            Voice,
            DirectMessage,
            GroupDm,
        }

        /// Server's Channel model
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Channel {
            pub id: Uuid,
            pub server_id: Option<Uuid>,
            pub name: String,
            pub topic: Option<String>,
            pub channel_type: ChannelType,
            pub position: i32,
            pub created_at: DateTime<Utc>,
            pub updated_at: DateTime<Utc>,
        }
    }

    /// Test that server's ChannelType serializes correctly as snake_case.
    #[test]
    fn test_channel_type_serialization_is_snake_case() {
        // Server serializes ChannelType with snake_case
        let server_channel_type = server_types::ChannelType::Text;
        let server_json = serde_json::to_string(&server_channel_type).unwrap();

        // Server produces "text" (snake_case) which matches client expectation
        assert_eq!(server_json, r#""text""#, "Server serializes as 'text' (snake_case)");

        // Client can deserialize it
        let client_result: ChannelType = serde_json::from_str(&server_json).unwrap();
        assert_eq!(client_result, ChannelType::Text);
    }

    /// Test that client's ChannelType serializes correctly as snake_case.
    #[test]
    fn test_channel_type_expected_serialization() {
        // Client's ChannelType uses snake_case
        let client_channel_type = ChannelType::Text;
        let client_json = serde_json::to_string(&client_channel_type).unwrap();

        // Client produces snake_case
        assert_eq!(client_json, r#""text""#, "Client produces 'text' (snake_case)");

        // Round-trip should work with snake_case
        let roundtrip: ChannelType = serde_json::from_str(&client_json).unwrap();
        assert_eq!(roundtrip, ChannelType::Text);
    }

    /// Test that a full server Channel can be deserialized as ChannelData.
    #[test]
    fn test_full_channel_deserialization_succeeds() {
        let server_channel = server_types::Channel {
            id: Uuid::new_v4(),
            server_id: Some(Uuid::new_v4()),
            name: "general".to_string(),
            topic: Some("General discussion".to_string()),
            channel_type: server_types::ChannelType::Text,
            position: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let server_json = serde_json::to_string(&server_channel).unwrap();

        // The JSON should contain "text" (snake_case)
        assert!(
            server_json.contains(r#""channel_type":"text""#),
            "Server JSON should contain 'text' (snake_case). Got: {}",
            server_json
        );

        // Deserialization as ChannelData should succeed
        let channel_data: ChannelData = serde_json::from_str(&server_json)
            .expect("Server Channel JSON should deserialize as ChannelData");

        assert_eq!(channel_data.name, "general");
        assert_eq!(channel_data.channel_type, ChannelType::Text);
    }

    /// Test all channel type variants for correct snake_case serialization.
    #[test]
    fn test_all_channel_types_snake_case() {
        // Text
        let json = serde_json::to_string(&ChannelType::Text).unwrap();
        assert_eq!(json, r#""text""#);

        // Voice
        let json = serde_json::to_string(&ChannelType::Voice).unwrap();
        assert_eq!(json, r#""voice""#);

        // DirectMessage -> direct_message
        let json = serde_json::to_string(&ChannelType::DirectMessage).unwrap();
        assert_eq!(json, r#""direct_message""#);

        // GroupDm -> group_dm
        let json = serde_json::to_string(&ChannelType::GroupDm).unwrap();
        assert_eq!(json, r#""group_dm""#);
    }

    /// Test server-client round-trip compatibility for all channel types.
    #[test]
    fn test_server_client_round_trip() {
        // Test all variants
        let variants = [
            (server_types::ChannelType::Text, ChannelType::Text),
            (server_types::ChannelType::Voice, ChannelType::Voice),
            (server_types::ChannelType::DirectMessage, ChannelType::DirectMessage),
            (server_types::ChannelType::GroupDm, ChannelType::GroupDm),
        ];

        for (server_type, expected_client_type) in variants {
            let json = serde_json::to_string(&server_type).unwrap();
            let client_type: ChannelType = serde_json::from_str(&json).unwrap();
            assert_eq!(client_type, expected_client_type, "Failed for {:?}", server_type);
        }
    }
}
