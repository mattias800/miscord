use miscord_protocol::ServerMessage;
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub user_id: Uuid,
    pub subscribed_channels: HashSet<Uuid>,
    pub subscribed_threads: HashSet<Uuid>,
}

pub struct ConnectionManager {
    /// Map from connection ID to message sender channel
    senders: RwLock<HashMap<Uuid, mpsc::UnboundedSender<String>>>,
    /// Map from connection ID to connection info
    connection_info: RwLock<HashMap<Uuid, ConnectionInfo>>,
    /// Map from user ID to connection IDs (a user may have multiple connections)
    user_connections: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
    /// Map from channel ID to connection IDs subscribed to that channel
    channel_subscribers: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
    /// Map from thread parent message ID to connection IDs subscribed to that thread
    thread_subscribers: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            senders: RwLock::new(HashMap::new()),
            connection_info: RwLock::new(HashMap::new()),
            user_connections: RwLock::new(HashMap::new()),
            channel_subscribers: RwLock::new(HashMap::new()),
            thread_subscribers: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add_connection(
        &self,
        connection_id: Uuid,
        user_id: Uuid,
        sender: mpsc::UnboundedSender<String>,
    ) {
        self.senders.write().await.insert(connection_id, sender);

        self.connection_info.write().await.insert(
            connection_id,
            ConnectionInfo {
                user_id,
                subscribed_channels: HashSet::new(),
                subscribed_threads: HashSet::new(),
            },
        );

        self.user_connections
            .write()
            .await
            .entry(user_id)
            .or_default()
            .insert(connection_id);

        tracing::debug!(
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

            // Remove from thread subscribers
            for thread_id in &info.subscribed_threads {
                if let Some(subs) = self.thread_subscribers.write().await.get_mut(thread_id) {
                    subs.remove(&connection_id);
                }
            }

            tracing::debug!(
                "User {} disconnected (connection ID {})",
                info.user_id,
                connection_id
            );
        }

        self.senders.write().await.remove(&connection_id);
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

        tracing::debug!("Connection {} subscribed to channel {}", connection_id, channel_id);
    }

    pub async fn unsubscribe_from_channel(&self, connection_id: Uuid, channel_id: Uuid) {
        if let Some(info) = self.connection_info.write().await.get_mut(&connection_id) {
            info.subscribed_channels.remove(&channel_id);
        }

        if let Some(subs) = self.channel_subscribers.write().await.get_mut(&channel_id) {
            subs.remove(&connection_id);
        }
    }

    pub async fn subscribe_to_thread(&self, connection_id: Uuid, thread_id: Uuid) {
        if let Some(info) = self.connection_info.write().await.get_mut(&connection_id) {
            info.subscribed_threads.insert(thread_id);
        }

        self.thread_subscribers
            .write()
            .await
            .entry(thread_id)
            .or_default()
            .insert(connection_id);

        tracing::debug!("Connection {} subscribed to thread {}", connection_id, thread_id);
    }

    pub async fn unsubscribe_from_thread(&self, connection_id: Uuid, thread_id: Uuid) {
        if let Some(info) = self.connection_info.write().await.get_mut(&connection_id) {
            info.subscribed_threads.remove(&thread_id);
        }

        if let Some(subs) = self.thread_subscribers.write().await.get_mut(&thread_id) {
            subs.remove(&connection_id);
        }

        tracing::debug!("Connection {} unsubscribed from thread {}", connection_id, thread_id);
    }

    pub async fn broadcast_to_thread(&self, thread_id: Uuid, message: &ServerMessage) {
        let json = match serde_json::to_string(message) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize message: {}", e);
                return;
            }
        };

        let subscribers = self.thread_subscribers.read().await;
        let senders = self.senders.read().await;

        if let Some(subs) = subscribers.get(&thread_id) {
            tracing::debug!("Broadcasting to {} subscribers of thread {}", subs.len(), thread_id);
            for conn_id in subs {
                if let Some(sender) = senders.get(conn_id) {
                    if let Err(e) = sender.send(json.clone()) {
                        tracing::error!("Failed to send message to {}: {}", conn_id, e);
                    }
                }
            }
        } else {
            tracing::debug!("No subscribers for thread {}", thread_id);
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
        let senders = self.senders.read().await;

        if let Some(subs) = subscribers.get(&channel_id) {
            tracing::debug!("Broadcasting to {} subscribers of channel {}", subs.len(), channel_id);
            for conn_id in subs {
                if let Some(sender) = senders.get(conn_id) {
                    if let Err(e) = sender.send(json.clone()) {
                        tracing::error!("Failed to send message to {}: {}", conn_id, e);
                    }
                }
            }
        } else {
            tracing::debug!("No subscribers for channel {}", channel_id);
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
        let senders = self.senders.read().await;

        if let Some(conn_ids) = user_connections.get(&user_id) {
            tracing::debug!(
                "send_to_user: user_id={}, connections={:?}",
                user_id,
                conn_ids
            );
            for conn_id in conn_ids {
                if let Some(sender) = senders.get(conn_id) {
                    if let Err(e) = sender.send(json.clone()) {
                        tracing::error!("Failed to send message to user {} ({}): {}", user_id, conn_id, e);
                    }
                }
            }
        } else {
            tracing::warn!("send_to_user: user_id={} not found in user_connections", user_id);
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

        // Debug: log which user this connection belongs to
        let connection_info = self.connection_info.read().await;
        if let Some(info) = connection_info.get(&connection_id) {
            tracing::debug!(
                "send_to_connection: connection_id={}, belongs to user={}",
                connection_id,
                info.user_id
            );
        } else {
            tracing::warn!(
                "send_to_connection: connection_id={} not found in connection_info!",
                connection_id
            );
        }
        drop(connection_info);

        let senders = self.senders.read().await;

        if let Some(sender) = senders.get(&connection_id) {
            if let Err(e) = sender.send(json) {
                tracing::error!("Failed to send message to {}: {}", connection_id, e);
            }
        } else {
            tracing::warn!(
                "send_to_connection: no sender found for connection_id={}",
                connection_id
            );
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
