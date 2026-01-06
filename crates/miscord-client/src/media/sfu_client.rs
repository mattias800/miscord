//! SFU Client for WebRTC video streaming
//!
//! Handles WebRTC peer connection to SFU server, sends local video,
//! and receives remote video streams.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_remote::TrackRemote;

use super::gst_encoder::{GstVp8Decoder, GstVp8Encoder};
use super::gst_video::VideoFrame;

/// Remote video frame from another user
#[derive(Clone)]
pub struct RemoteVideoFrame {
    pub user_id: Uuid,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGB data
}

/// Channel for sending ICE candidates to the signaling layer
pub type IceCandidateSender = mpsc::UnboundedSender<IceCandidate>;

/// ICE candidate for signaling
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

/// SFU Client manages WebRTC connection to the SFU server
#[derive(Clone)]
pub struct SfuClient {
    peer_connection: Arc<RwLock<Option<Arc<RTCPeerConnection>>>>,
    local_video_track: Arc<RwLock<Option<Arc<TrackLocalStaticSample>>>>,
    remote_frames: Arc<RwLock<HashMap<Uuid, RemoteVideoFrame>>>,
    ice_candidate_tx: Arc<RwLock<Option<IceCandidateSender>>>,
    channel_id: Arc<RwLock<Option<Uuid>>>,
    /// VP8 encoder for local video
    encoder: Arc<Mutex<Option<GstVp8Encoder>>>,
}

impl SfuClient {
    /// Create a new SFU client
    pub fn new() -> Self {
        Self {
            peer_connection: Arc::new(RwLock::new(None)),
            local_video_track: Arc::new(RwLock::new(None)),
            remote_frames: Arc::new(RwLock::new(HashMap::new())),
            ice_candidate_tx: Arc::new(RwLock::new(None)),
            channel_id: Arc::new(RwLock::new(None)),
            encoder: Arc::new(Mutex::new(None)),
        }
    }

    /// Connect to the SFU server
    /// Returns the SDP offer to send to the server via WebSocket
    pub async fn connect(
        &self,
        channel_id: Uuid,
        ice_servers: Vec<(String, Option<String>, Option<String>)>,
        ice_candidate_tx: IceCandidateSender,
    ) -> Result<String> {
        // Disconnect if already connected
        if self.peer_connection.read().await.is_some() {
            self.disconnect().await?;
        }

        *self.channel_id.write().await = Some(channel_id);
        *self.ice_candidate_tx.write().await = Some(ice_candidate_tx.clone());

        // Create media engine with VP8 codec
        let mut media_engine = MediaEngine::default();
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: "video/VP8".to_string(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line: String::new(),
                    rtcp_feedback: vec![],
                },
                payload_type: 96,
                ..Default::default()
            },
            RTPCodecType::Video,
        )?;

        // Create interceptor registry
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)?;

        // Build API
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        // Build ICE servers config
        let ice_server_configs: Vec<RTCIceServer> = ice_servers
            .into_iter()
            .map(|(url, username, credential)| RTCIceServer {
                urls: vec![url],
                username: username.unwrap_or_default(),
                credential: credential.unwrap_or_default(),
                ..Default::default()
            })
            .collect();

        let config = RTCConfiguration {
            ice_servers: ice_server_configs,
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);

        // Create local video track
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/VP8".to_string(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: vec![],
            },
            "video".to_string(),
            "stream-local".to_string(),
        ));

        // Add track to peer connection
        peer_connection.add_track(video_track.clone()).await?;

        // Note: We don't add a receive transceiver here.
        // The server will add tracks and send a renegotiation offer,
        // which will properly set up receive transceivers.

        // Set up ICE candidate handler
        let ice_tx = ice_candidate_tx;
        peer_connection.on_ice_candidate(Box::new(move |candidate| {
            let tx = ice_tx.clone();
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    if let Ok(json) = candidate.to_json() {
                        let _ = tx.send(IceCandidate {
                            candidate: json.candidate,
                            sdp_mid: json.sdp_mid,
                            sdp_mline_index: json.sdp_mline_index,
                        });
                    }
                }
            })
        }));

        // Set up track handler for incoming remote video
        let remote_frames = self.remote_frames.clone();
        tracing::info!("Registering on_track callback for incoming remote video");
        peer_connection.on_track(Box::new(move |track, _receiver, _transceiver| {
            tracing::info!(
                "on_track callback fired! Track ID: {}, Stream ID: {}, Kind: {:?}",
                track.id(),
                track.stream_id(),
                track.kind()
            );
            let frames = remote_frames.clone();
            Box::pin(async move {
                handle_remote_track(track, frames).await;
            })
        }));

        // Create offer
        let offer = peer_connection.create_offer(None).await?;

        // Set local description
        peer_connection.set_local_description(offer.clone()).await?;

        *self.peer_connection.write().await = Some(peer_connection);
        *self.local_video_track.write().await = Some(video_track);

        tracing::info!("SFU client connected, offer created");

        Ok(offer.sdp)
    }

    /// Handle answer from SFU server
    pub async fn handle_answer(&self, sdp: String) -> Result<()> {
        let pc_guard = self.peer_connection.read().await;
        let pc = pc_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        let answer = RTCSessionDescription::answer(sdp)?;
        pc.set_remote_description(answer).await?;

        tracing::info!("SFU answer processed");
        Ok(())
    }

    /// Handle renegotiation offer from server (when new tracks are added)
    /// Returns Ok(Some(answer_sdp)) on success, Ok(None) if not ready (caller should retry),
    /// or Err on failure
    pub async fn handle_renegotiate(&self, sdp: String) -> Result<Option<String>> {
        use webrtc::peer_connection::signaling_state::RTCSignalingState;

        // Log the incoming SDP for debugging
        tracing::info!("Received renegotiation SDP, length: {}", sdp.len());
        // Log first part of SDP to see media lines
        for line in sdp.lines().take(30) {
            if line.starts_with("m=") || line.starts_with("a=mid") || line.starts_with("a=msid") || line.starts_with("a=ssrc") {
                tracing::info!("SDP line: {}", line);
            }
        }

        let pc_guard = self.peer_connection.read().await;
        let pc = pc_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        // Check if we're in stable state - if not, caller should retry later
        if pc.signaling_state() != RTCSignalingState::Stable {
            tracing::debug!(
                "Not ready for renegotiation, signaling state: {:?}",
                pc.signaling_state()
            );
            return Ok(None);
        }

        tracing::info!("Processing renegotiation in stable state");

        let offer = RTCSessionDescription::offer(sdp)?;
        pc.set_remote_description(offer).await?;
        tracing::info!("Remote description set, on_track should have fired for new tracks");

        // Create answer
        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer.clone()).await?;

        tracing::info!("SFU renegotiation processed, answer created");
        Ok(Some(answer.sdp))
    }

    /// Add ICE candidate from server
    pub async fn add_ice_candidate(
        &self,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    ) -> Result<()> {
        let pc_guard = self.peer_connection.read().await;
        let pc = pc_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        let ice_candidate = RTCIceCandidateInit {
            candidate,
            sdp_mid,
            sdp_mline_index,
            ..Default::default()
        };

        pc.add_ice_candidate(ice_candidate).await?;
        Ok(())
    }

    /// Send a video frame to the SFU
    /// The frame should be RGB data that will be encoded to VP8
    pub async fn send_frame(&self, frame: &VideoFrame) -> Result<()> {
        let track_guard = self.local_video_track.read().await;
        let track = track_guard
            .as_ref()
            .ok_or_else(|| anyhow!("No local track"))?;

        // Encode the RGB frame to VP8 using GStreamer
        let encoded = {
            let mut encoder_guard = self.encoder.lock().map_err(|e| anyhow!("Encoder lock error: {}", e))?;

            // Create encoder on first frame (lazy initialization)
            // This allows us to get the actual frame dimensions
            if encoder_guard.is_none() {
                tracing::info!("Creating VP8 encoder for {}x{}", frame.width, frame.height);
                match GstVp8Encoder::new(frame.width, frame.height) {
                    Ok(enc) => {
                        *encoder_guard = Some(enc);
                    }
                    Err(e) => {
                        tracing::error!("Failed to create VP8 encoder: {}", e);
                        return Err(anyhow!("Failed to create VP8 encoder: {}", e));
                    }
                }
            }

            let encoder = encoder_guard.as_ref().unwrap();

            // Check if dimensions match, recreate encoder if needed
            if encoder.width() != frame.width || encoder.height() != frame.height {
                tracing::info!("Frame size changed to {}x{}, recreating encoder", frame.width, frame.height);
                drop(encoder_guard);
                let mut encoder_guard = self.encoder.lock().map_err(|e| anyhow!("Encoder lock error: {}", e))?;
                *encoder_guard = Some(GstVp8Encoder::new(frame.width, frame.height)?);
                encoder_guard.as_ref().unwrap().encode(frame)?
            } else {
                encoder.encode(frame)?
            }
        };

        // Skip if encoder returned empty (still buffering)
        if encoded.is_empty() {
            return Ok(());
        }

        // Create RTP sample
        use webrtc::media::Sample;
        let sample = Sample {
            data: encoded.into(),
            duration: std::time::Duration::from_millis(33), // ~30 fps
            ..Default::default()
        };

        track.write_sample(&sample).await?;
        Ok(())
    }

    /// Get the latest frame from a remote user
    pub async fn get_remote_frame(&self, user_id: Uuid) -> Option<RemoteVideoFrame> {
        self.remote_frames.read().await.get(&user_id).cloned()
    }

    /// Get all remote user IDs with available frames
    pub async fn get_remote_users(&self) -> Vec<Uuid> {
        self.remote_frames.read().await.keys().cloned().collect()
    }

    /// Disconnect from the SFU
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(pc) = self.peer_connection.write().await.take() {
            pc.close().await?;
        }
        *self.local_video_track.write().await = None;
        *self.channel_id.write().await = None;
        *self.ice_candidate_tx.write().await = None;
        self.remote_frames.write().await.clear();

        // Clean up encoder
        if let Ok(mut encoder) = self.encoder.lock() {
            *encoder = None;
        }

        tracing::info!("SFU client disconnected");
        Ok(())
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        self.peer_connection.read().await.is_some()
    }
}

impl Default for SfuClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle incoming remote track from another user
async fn handle_remote_track(
    track: Arc<TrackRemote>,
    remote_frames: Arc<RwLock<HashMap<Uuid, RemoteVideoFrame>>>,
) {
    let stream_id = track.stream_id().to_string();

    // Parse user ID from stream ID (format: "stream-{user_id}")
    let user_id = match stream_id.strip_prefix("stream-") {
        Some(id_str) => match Uuid::parse_str(id_str) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!("Invalid user ID in stream ID: {}", stream_id);
                return;
            }
        },
        None => {
            tracing::warn!("Invalid stream ID format: {}", stream_id);
            return;
        }
    };

    tracing::info!(
        "Receiving remote video track from user {}, track: {}",
        user_id,
        track.id()
    );

    // Wait briefly for the track's RTP receiver to be fully initialized
    // This is needed because the on_track callback can fire before the
    // internal receiver is ready (reduced from 500ms for lower latency)
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Create VP8 decoder for this track
    let decoder = match GstVp8Decoder::new() {
        Ok(dec) => dec,
        Err(e) => {
            tracing::error!("Failed to create VP8 decoder for user {}: {}", user_id, e);
            return;
        }
    };

    // Read RTP packets and decode to frames
    tracing::info!("Starting RTP read loop for user {}", user_id);
    let mut packet_count = 0u64;
    loop {
        match track.read_rtp().await {
            Ok((rtp_packet, _attributes)) => {
                packet_count += 1;
                if packet_count % 100 == 1 {
                    tracing::info!("Received RTP packet {} for user {}, payload size: {}",
                        packet_count, user_id, rtp_packet.payload.len());
                }

                // Serialize full RTP packet for rtpvp8depay
                use webrtc::util::Marshal;
                let rtp_bytes = match rtp_packet.marshal() {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::warn!("Failed to marshal RTP packet: {}", e);
                        continue;
                    }
                };

                // Decode VP8 packet to frame using GStreamer
                match decoder.decode(&rtp_bytes) {
                    Ok(Some(frame)) => {
                        tracing::info!("Decoded frame for user {}: {}x{}", user_id, frame.width, frame.height);
                        let remote_frame = RemoteVideoFrame {
                            user_id,
                            width: frame.width,
                            height: frame.height,
                            data: frame.data,
                        };
                        remote_frames.write().await.insert(user_id, remote_frame);
                    }
                    Ok(None) => {
                        // Decoder still buffering, no frame yet
                    }
                    Err(e) => {
                        tracing::warn!("VP8 decode error for user {}: {}", user_id, e);
                    }
                }
            }
            Err(e) => {
                if e.to_string().contains("closed") {
                    tracing::info!("Remote track closed for user {}", user_id);
                    break;
                }
                tracing::warn!("Error reading RTP from remote track: {}", e);
            }
        }
    }

    // Remove from remote frames when track ends
    remote_frames.write().await.remove(&user_id);
}

