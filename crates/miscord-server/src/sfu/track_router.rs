//! Track Router for zero-copy RTP forwarding
//!
//! Routes RTP packets from a publisher's track to all subscribers
//! without any processing - just raw packet forwarding.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocalWriter;
use webrtc::track::track_remote::TrackRemote;

/// Routes RTP packets from a source track to multiple subscriber tracks
pub struct TrackRouter {
    /// The source track from the publisher
    source_track: Arc<TrackRemote>,
    /// ID of the user publishing this track
    publisher_id: Uuid,
    /// Track ID for identification
    track_id: String,
    /// Local tracks for each subscriber
    subscriber_tracks: RwLock<HashMap<Uuid, Arc<TrackLocalStaticRTP>>>,
    /// Whether the router is active
    active: Arc<RwLock<bool>>,
}

impl TrackRouter {
    /// Create a new track router for a source track
    pub fn new(source_track: Arc<TrackRemote>, publisher_id: Uuid, track_id: String) -> Self {
        Self {
            source_track,
            publisher_id,
            track_id,
            subscriber_tracks: RwLock::new(HashMap::new()),
            active: Arc::new(RwLock::new(true)),
        }
    }

    /// Get the publisher ID
    pub fn publisher_id(&self) -> Uuid {
        self.publisher_id
    }

    /// Get the track ID
    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    /// Get codec info from source track
    pub fn codec(&self) -> RTPCodecType {
        self.source_track.kind()
    }

    /// Start the forwarding loop
    /// This reads RTP packets from the source and writes them to all subscribers
    pub async fn start_forwarding(self: Arc<Self>) {
        tracing::info!(
            "Starting RTP forwarding for track {} from user {}",
            self.track_id,
            self.publisher_id
        );

        let mut packet_count = 0u64;
        loop {
            // Check if still active (less frequently to reduce lock contention)
            if packet_count % 100 == 0 && !*self.active.read().await {
                tracing::info!("Track router {} stopped", self.track_id);
                break;
            }

            // Read RTP packet from source
            match self.source_track.read_rtp().await {
                Ok((rtp_packet, _attributes)) => {
                    packet_count += 1;

                    // Log every 100 packets
                    if packet_count % 100 == 1 {
                        tracing::info!(
                            "Forwarding RTP packet {} from {} (payload: {} bytes)",
                            packet_count,
                            self.publisher_id,
                            rtp_packet.payload.len()
                        );
                    }

                    // Forward to all subscribers (zero-copy: just the packet bytes)
                    let subscribers = self.subscriber_tracks.read().await;
                    let subscriber_count = subscribers.len();

                    for (subscriber_id, local_track) in subscribers.iter() {
                        if let Err(e) = local_track.write_rtp(&rtp_packet).await {
                            tracing::warn!(
                                "Failed to forward RTP to subscriber {}: {}",
                                subscriber_id,
                                e
                            );
                        }
                    }

                    // Log subscriber info occasionally
                    if packet_count % 100 == 1 {
                        tracing::debug!("Forwarded to {} subscribers", subscriber_count);
                    }
                }
                Err(e) => {
                    // Check if error is due to track being closed
                    if e.to_string().contains("closed") {
                        tracing::info!("Source track closed for {}", self.track_id);
                        break;
                    }
                    tracing::warn!("Error reading RTP from source track: {}", e);
                }
            }
        }

        // Mark as inactive
        *self.active.write().await = false;
    }

    /// Add a subscriber to receive forwarded RTP packets
    pub async fn add_subscriber(&self, subscriber_id: Uuid) -> Arc<TrackLocalStaticRTP> {
        // Create a local track for this subscriber with the same codec as source
        let codec = self.source_track.codec();

        let local_track = Arc::new(TrackLocalStaticRTP::new(
            codec.capability.clone(),
            format!("{}-{}", self.track_id, subscriber_id),
            format!("stream-{}", self.publisher_id),
        ));

        self.subscriber_tracks
            .write()
            .await
            .insert(subscriber_id, local_track.clone());

        tracing::info!(
            "Added subscriber {} to track {} (publisher: {})",
            subscriber_id,
            self.track_id,
            self.publisher_id
        );

        local_track
    }

    /// Remove a subscriber
    pub async fn remove_subscriber(&self, subscriber_id: Uuid) -> bool {
        let removed = self
            .subscriber_tracks
            .write()
            .await
            .remove(&subscriber_id)
            .is_some();

        if removed {
            tracing::info!(
                "Removed subscriber {} from track {}",
                subscriber_id,
                self.track_id
            );
        }

        removed
    }

    /// Get the number of subscribers
    pub async fn subscriber_count(&self) -> usize {
        self.subscriber_tracks.read().await.len()
    }

    /// Stop the forwarding loop
    pub async fn stop(&self) {
        *self.active.write().await = false;
        tracing::info!("Track router {} marked for stop", self.track_id);
    }

    /// Check if router is still active
    pub async fn is_active(&self) -> bool {
        *self.active.read().await
    }
}
