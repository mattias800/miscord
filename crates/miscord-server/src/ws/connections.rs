use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use miscord_protocol::ServerMessage;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use uuid::Uuid;

type WsSender = SplitSink<WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>, Message>;

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub user_id: Uuid,
    pub subscribed_channels: HashSet<Uuid>,
}

pub struct ConnectionManager {
    /// Map from connection ID to WebSocket sender
    connections: RwLock<HashMap<Uuid, Arc<RwLock<WsSender>>>>,
    /// Map from connection ID to connection info
    connection_info: RwLock<HashMap<Uuid, ConnectionInfo>>,
    /// Map from user ID to connection IDs (a user may have multiple connections)
    user_connections: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
    /// Map from channel ID to connection IDs subscribed to that channel
    channel_subscribers: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            connection_info: RwLock::new(HashMap::new()),
            user_connections: RwLock::new(HashMap::new()),
            channel_subscribers: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add_connection(
        &self,
        connection_id: Uuid,
        user_id: Uuid,
        sender: WsSender,
    ) {
        let sender = Arc::new(RwLock::new(sender));

        self.connections
            .write()
            .await
            .insert(connection_id, sender);

        self.connection_info.write().await.insert(
            connection_id,
            ConnectionInfo {
                user_id,
                subscribed_channels: HashSet::new(),
            },
        );

        self.user_connections
            .write()
            .await
            .entry(user_id)
            .or_default()
            .insert(connection_id);

        tracing::info!(
            "User {} connected with connection ID {}",
            user_id,
            connection_id
        );
    }

    pub async fn remove_connection(&self, connection_id: Uuid) {
        // Get connection info before removing
        let info = self.connection_info.write().await.remove(&connection_id);

        if let Some(info) = info {
            // Remove from user connections
            if let Some(user_conns) = self.user_connections.write().await.get_mut(&info.user_id) {
                user_conns.remove(&connection_id);
            }

            // Remove from channel subscribers
            for channel_id in &info.subscribed_channels {
                if let Some(subs) = self.channel_subscribers.write().await.get_mut(channel_id) {
                    subs.remove(&connection_id);
                }
            }

            tracing::info!(
                "User {} disconnected (connection ID {})",
                info.user_id,
                connection_id
            );
        }

        self.connections.write().await.remove(&connection_id);
    }

    pub async fn subscribe_to_channel(&self, connection_id: Uuid, channel_id: Uuid) {
        if let Some(info) = self.connection_info.write().await.get_mut(&connection_id) {
            info.subscribed_channels.insert(channel_id);
        }

        self.channel_subscribers
            .write()
            .await
            .entry(channel_id)
            .or_default()
            .insert(connection_id);
    }

    pub async fn unsubscribe_from_channel(&self, connection_id: Uuid, channel_id: Uuid) {
        if let Some(info) = self.connection_info.write().await.get_mut(&connection_id) {
            info.subscribed_channels.remove(&channel_id);
        }

        if let Some(subs) = self.channel_subscribers.write().await.get_mut(&channel_id) {
            subs.remove(&connection_id);
        }
    }

    pub async fn broadcast_to_channel(&self, channel_id: Uuid, message: &ServerMessage) {
        let json = match serde_json::to_string(message) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize message: {}", e);
                return;
            }
        };

        let subscribers = self.channel_subscribers.read().await;
        let connections = self.connections.read().await;

        if let Some(subs) = subscribers.get(&channel_id) {
            for conn_id in subs {
                if let Some(sender) = connections.get(conn_id) {
                    let mut sender = sender.write().await;
                    if let Err(e) = sender.send(Message::Text(json.clone().into())).await {
                        tracing::error!("Failed to send message to {}: {}", conn_id, e);
                    }
                }
            }
        }
    }

    pub async fn send_to_user(&self, user_id: Uuid, message: &ServerMessage) {
        let json = match serde_json::to_string(message) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize message: {}", e);
                return;
            }
        };

        let user_connections = self.user_connections.read().await;
        let connections = self.connections.read().await;

        if let Some(conn_ids) = user_connections.get(&user_id) {
            for conn_id in conn_ids {
                if let Some(sender) = connections.get(conn_id) {
                    let mut sender = sender.write().await;
                    if let Err(e) = sender.send(Message::Text(json.clone().into())).await {
                        tracing::error!("Failed to send message to user {} ({}): {}", user_id, conn_id, e);
                    }
                }
            }
        }
    }

    pub async fn send_to_connection(&self, connection_id: Uuid, message: &ServerMessage) {
        let json = match serde_json::to_string(message) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize message: {}", e);
                return;
            }
        };

        let connections = self.connections.read().await;

        if let Some(sender) = connections.get(&connection_id) {
            let mut sender = sender.write().await;
            if let Err(e) = sender.send(Message::Text(json.clone().into())).await {
                tracing::error!("Failed to send message to {}: {}", connection_id, e);
            }
        }
    }

    pub async fn get_online_users(&self) -> Vec<Uuid> {
        self.user_connections
            .read()
            .await
            .keys()
            .copied()
            .collect()
    }

    pub async fn is_user_online(&self, user_id: Uuid) -> bool {
        self.user_connections
            .read()
            .await
            .get(&user_id)
            .map(|conns| !conns.is_empty())
            .unwrap_or(false)
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}
