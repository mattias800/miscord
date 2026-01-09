//! Track Router for RTP forwarding
//!
//! Routes RTP packets from a publisher's track to all subscribers.
//! Uses TrackLocalStaticRTP for direct RTP forwarding to preserve H.264 packetization.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocalWriter;
use webrtc::track::track_remote::TrackRemote;

use miscord_protocol::TrackType;

/// Routes RTP packets from a source track to multiple subscriber tracks
pub struct TrackRouter {
    /// The source track from the publisher
    source_track: Arc<TrackRemote>,
    /// ID of the user publishing this track
    publisher_id: Uuid,
    /// Track ID for identification
    track_id: String,
    /// Type of track (webcam or screen)
    track_type: TrackType,
    /// Local tracks for each subscriber (using TrackLocalStaticRTP for direct forwarding)
    subscriber_tracks: RwLock<HashMap<Uuid, Arc<TrackLocalStaticRTP>>>,
    /// Whether the router is active
    active: Arc<RwLock<bool>>,
}

impl TrackRouter {
    /// Create a new track router for a source track
    pub fn new(
        source_track: Arc<TrackRemote>,
        publisher_id: Uuid,
        track_id: String,
        track_type: TrackType,
    ) -> Self {
        Self {
            source_track,
            publisher_id,
            track_id,
            track_type,
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

    /// Get the track type
    pub fn track_type(&self) -> TrackType {
        self.track_type
    }

    /// Get codec info from source track
    pub fn codec(&self) -> RTPCodecType {
        self.source_track.kind()
    }

    /// Start the forwarding loop
    /// This reads RTP packets from the source and forwards them directly to all subscribers
    /// Using direct RTP forwarding preserves H.264 packetization (FU-A fragmentation)
    pub async fn start_forwarding(self: Arc<Self>) {
        tracing::info!(
            "Starting RTP forwarding for track {} from user {}",
            self.track_id,
            self.publisher_id
        );

        // Wait for the track receiver to be ready
        // The RTPReceiver may not be attached immediately after track detection
        let mut retry_count = 0;
        let max_retries = 50; // 5 seconds max wait

        loop {
            if !*self.active.read().await {
                tracing::info!("Track router {} stopped before receiving packets", self.track_id);
                return;
            }

            // Add timeout to diagnose blocking read_rtp
            let read_result = tokio::time::timeout(
                tokio::time::Duration::from_secs(2),
                self.source_track.read_rtp()
            ).await;

            match read_result {
                Err(_) => {
                    // Timeout - read_rtp is blocking
                    retry_count += 1;
                    if retry_count == 1 || retry_count % 5 == 0 {
                        tracing::warn!(
                            "Track {} read_rtp timeout (attempt {}) - no packets arriving",
                            self.track_id,
                            retry_count
                        );
                    }
                    if retry_count > 30 {
                        tracing::error!("Track {} never received packets after {} timeouts", self.track_id, retry_count);
                        *self.active.write().await = false;
                        return;
                    }
                    continue;
                }
                Ok(Ok((rtp_packet, _attributes))) => {
                    // First packet received - track is ready
                    tracing::info!(
                        "Track {} ready - received first RTP packet (payload: {} bytes)",
                        self.track_id,
                        rtp_packet.payload.len()
                    );

                    // Forward this first packet and continue with the main loop
                    self.forward_packet(&rtp_packet, 1).await;
                    break;
                }
                Ok(Err(e)) => {
                    let error_msg = e.to_string();

                    // Track closed - stop
                    if error_msg.contains("closed") {
                        tracing::info!("Source track closed for {} before receiving packets", self.track_id);
                        *self.active.write().await = false;
                        return;
                    }

                    // RTPReceiver not ready - wait and retry
                    if error_msg.contains("RTPReceiver must not be nil") {
                        retry_count += 1;
                        if retry_count > max_retries {
                            tracing::error!(
                                "Track {} never became ready after {} retries",
                                self.track_id,
                                max_retries
                            );
                            *self.active.write().await = false;
                            return;
                        }

                        if retry_count == 1 || retry_count % 10 == 0 {
                            tracing::info!(
                                "Waiting for track {} receiver to be ready (attempt {})",
                                self.track_id,
                                retry_count
                            );
                        }

                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        continue;
                    }

                    // Other error - log and continue trying
                    tracing::warn!("Error reading RTP from source track {}: {}", self.track_id, e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }
        }

        // Main forwarding loop
        let mut packet_count = 1u64; // Already forwarded one packet

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
                    self.forward_packet(&rtp_packet, packet_count).await;
                }
                Err(e) => {
                    let error_msg = e.to_string();

                    // Check if error is due to track being closed
                    if error_msg.contains("closed") {
                        tracing::info!("Source track closed for {}", self.track_id);
                        break;
                    }

                    // RTPReceiver not ready - this can happen during renegotiation
                    // Just skip this iteration silently
                    if error_msg.contains("RTPReceiver must not be nil") {
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        continue;
                    }

                    // Log other errors (but only occasionally to avoid spam)
                    if packet_count % 100 == 1 {
                        tracing::warn!("Error reading RTP from source track: {}", e);
                    }
                }
            }
        }

        // Mark as inactive
        *self.active.write().await = false;
    }

    /// Forward an RTP packet to all subscribers
    async fn forward_packet(&self, rtp_packet: &webrtc::rtp::packet::Packet, packet_count: u64) {
        // LATENCY MEASUREMENT: Log server forward timestamp
        if packet_count % 30 == 1 {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            tracing::info!(
                "[LATENCY] SERVER_FWD track={} pkt={} ts={} seq={}",
                self.track_id,
                packet_count,
                ts,
                rtp_packet.header.sequence_number
            );
        }

        // Log every 100 packets
        if packet_count % 100 == 1 {
            tracing::info!(
                "Forwarding RTP packet {} from {} (payload: {} bytes, seq: {}, ts: {})",
                packet_count,
                self.publisher_id,
                rtp_packet.payload.len(),
                rtp_packet.header.sequence_number,
                rtp_packet.header.timestamp
            );
        }

        // Forward the complete RTP packet to all subscribers
        // This preserves H.264 FU-A fragmentation headers
        let subscribers = self.subscriber_tracks.read().await;
        let subscriber_count = subscribers.len();

        for (subscriber_id, local_track) in subscribers.iter() {
            if let Err(e) = local_track.write_rtp(rtp_packet).await {
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

    /// Add a subscriber to receive forwarded RTP packets
    pub async fn add_subscriber(&self, subscriber_id: Uuid) -> Arc<TrackLocalStaticRTP> {
        // Create a local track for this subscriber
        // IMPORTANT: We must use a codec capability that exactly matches what's registered
        // in the MediaEngine, not the capability from the source track (which may have
        // different fmtp parameters). Otherwise add_track fails with "no codecs".
        let source_codec = self.source_track.codec();
        tracing::info!(
            "Creating subscriber track for {} - source codec: {:?}",
            subscriber_id,
            source_codec.capability
        );

        // Use hardcoded H.264 capability matching the MediaEngine registration
        let h264_capability = webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
            mime_type: "video/H264".to_string(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .to_string(),
            rtcp_feedback: vec![],
        };

        // Stream ID format: stream-{user_id}-{track_type}
        // This allows clients to identify both the publisher and the track type
        // Using TrackLocalStaticRTP for direct RTP forwarding preserves H.264 packetization
        let local_track = Arc::new(TrackLocalStaticRTP::new(
            h264_capability,
            format!("{}-{}", self.track_id, subscriber_id),
            format!("stream-{}-{}", self.publisher_id, self.track_type),
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
