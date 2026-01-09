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

use miscord_protocol::TrackType;

use super::gst_encoder::{GstScreenEncoder, GstVp8Decoder, GstVp8Encoder};
use super::gst_video::VideoFrame;

/// Remote video frame from another user
#[derive(Clone)]
pub struct RemoteVideoFrame {
    pub user_id: Uuid,
    pub track_type: TrackType,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA data
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
    /// Local webcam video track
    local_video_track: Arc<RwLock<Option<Arc<TrackLocalStaticSample>>>>,
    /// Local screen share track
    local_screen_track: Arc<RwLock<Option<Arc<TrackLocalStaticSample>>>>,
    /// Remote frames keyed by (user_id, track_type)
    remote_frames: Arc<RwLock<HashMap<(Uuid, TrackType), RemoteVideoFrame>>>,
    /// Tracks that have handlers already started (to prevent duplicate handlers)
    handled_tracks: Arc<RwLock<std::collections::HashSet<(Uuid, TrackType)>>>,
    ice_candidate_tx: Arc<RwLock<Option<IceCandidateSender>>>,
    channel_id: Arc<RwLock<Option<Uuid>>>,
    /// VP8 encoder for local webcam video
    encoder: Arc<Mutex<Option<GstVp8Encoder>>>,
    /// Encoder for local screen share (higher bitrate, no downscaling)
    screen_encoder: Arc<Mutex<Option<GstScreenEncoder>>>,
}

impl SfuClient {
    /// Create a new SFU client
    pub fn new() -> Self {
        Self {
            peer_connection: Arc::new(RwLock::new(None)),
            local_video_track: Arc::new(RwLock::new(None)),
            local_screen_track: Arc::new(RwLock::new(None)),
            remote_frames: Arc::new(RwLock::new(HashMap::new())),
            handled_tracks: Arc::new(RwLock::new(std::collections::HashSet::new())),
            ice_candidate_tx: Arc::new(RwLock::new(None)),
            channel_id: Arc::new(RwLock::new(None)),
            encoder: Arc::new(Mutex::new(None)),
            screen_encoder: Arc::new(Mutex::new(None)),
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

        // Create media engine with H.264 codec (hardware accelerated)
        let mut media_engine = MediaEngine::default();
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

        // ULTRA LOW LATENCY: Use empty interceptor registry
        // Default interceptors include NACK (waits for retransmits) and other buffers
        // For game-streaming-like latency, we skip them entirely
        // Trade-off: No packet loss recovery, but much lower latency
        let registry = Registry::new();
        tracing::info!("Using minimal interceptor registry for ultra-low latency");

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

        // Create local video track with H.264 codec
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/H264".to_string(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_string(),
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
        use webrtc::ice_transport::ice_gathering_state::RTCIceGatheringState;
        use webrtc::peer_connection::signaling_state::RTCSignalingState;

        // Log the incoming SDP for debugging - show all m= lines
        tracing::info!("Received renegotiation SDP, length: {}", sdp.len());
        let m_lines: Vec<&str> = sdp.lines()
            .filter(|l| l.starts_with("m=") || l.starts_with("a=mid") || l.starts_with("a=msid") || l.starts_with("a=sendrecv") || l.starts_with("a=recvonly") || l.starts_with("a=sendonly"))
            .collect();
        for line in &m_lines {
            tracing::info!("Renegotiation SDP: {}", line);
        }
        tracing::info!("SDP contains {} m= lines", m_lines.iter().filter(|l| l.starts_with("m=")).count());

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

        // Check if ICE gathering is still in progress - if so, wait
        let ice_gathering_state = pc.ice_gathering_state();
        if ice_gathering_state == RTCIceGatheringState::Gathering {
            tracing::debug!(
                "Not ready for renegotiation, ICE gathering in progress: {:?}",
                ice_gathering_state
            );
            return Ok(None);
        }

        tracing::info!("Processing renegotiation in stable state");

        // Re-register on_track callback before processing renegotiation
        // webrtc-rs may not fire on_track for new tracks during renegotiation
        // unless the callback is re-registered
        let remote_frames_callback = self.remote_frames.clone();
        let handled_tracks_callback = self.handled_tracks.clone();
        tracing::info!("Re-registering on_track callback for renegotiation");
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let stream_id = track.stream_id().to_string();
            tracing::info!(
                "on_track FIRED during renegotiation! Track ID: {}, Stream ID: {}, Kind: {:?}",
                track.id(),
                stream_id,
                track.kind()
            );
            let frames = remote_frames_callback.clone();
            let handled = handled_tracks_callback.clone();
            Box::pin(async move {
                // Check if we already have a handler for this track
                if let Some((user_id, track_type)) = parse_stream_id(&stream_id) {
                    {
                        let mut handled_guard = handled.write().await;
                        if handled_guard.contains(&(user_id, track_type)) {
                            tracing::info!(
                                "on_track: Track already handled for user {} {:?}, skipping",
                                user_id, track_type
                            );
                            return;
                        }
                        handled_guard.insert((user_id, track_type));
                    }
                    tracing::info!("on_track: Starting handler for user {} {:?}", user_id, track_type);
                }
                handle_remote_track(track, frames).await;
            })
        }));

        let offer = RTCSessionDescription::offer(sdp)?;
        pc.set_remote_description(offer).await?;
        tracing::info!("Remote description set successfully");

        // Create answer
        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer.clone()).await?;

        // Workaround: on_track may not fire during renegotiation in webrtc-rs
        // Poll transceivers to find new remote tracks manually
        // Use a retry loop because tracks may take time to appear after SDP exchange
        let remote_frames = self.remote_frames.clone();
        let pc_clone = pc.clone();

        // Spawn background task to poll for new tracks with retries
        let handled_tracks = self.handled_tracks.clone();
        tokio::spawn(async move {
            let mut attempts = 0;
            let max_attempts = 20; // 2 seconds max wait
            let mut new_tracks_found = 0;

            while attempts < max_attempts {
                let transceivers = pc_clone.get_transceivers().await;

                if attempts == 0 {
                    tracing::info!("Checking {} transceivers for new remote tracks", transceivers.len());
                }

                for (idx, transceiver) in transceivers.iter().enumerate() {
                    let receiver = transceiver.receiver().await;
                    let tracks = receiver.tracks().await;
                    let mid = transceiver.mid();
                    let direction = transceiver.direction();
                    let current_dir = transceiver.current_direction();

                    if attempts == 0 || (attempts % 5 == 0 && tracks.is_empty()) {
                        tracing::info!(
                            "Transceiver {}: mid={:?}, direction={:?}, current={:?}, tracks={}",
                            idx, mid, direction, current_dir, tracks.len()
                        );
                    }

                    for track in tracks {
                        let stream_id = track.stream_id().to_string();
                        let track_id = track.id().to_string();

                        // Skip local tracks and tracks we're already handling
                        if stream_id.starts_with("stream-local") || track_id.is_empty() {
                            continue;
                        }

                        // Parse user ID and track type from stream ID
                        if let Some((user_id, track_type)) = parse_stream_id(&stream_id) {
                            // Check if we already started handling this track (globally across all renegotiations)
                            let already_handled = handled_tracks.read().await.contains(&(user_id, track_type));

                            if !already_handled {
                                // Mark as handled BEFORE spawning to prevent race conditions
                                handled_tracks.write().await.insert((user_id, track_type));

                                tracing::info!(
                                    "Found new remote track via transceiver polling (attempt {}): {} from stream {} (user: {}, type: {:?})",
                                    attempts + 1, track_id, stream_id, user_id, track_type
                                );

                                // Spawn handler for this track
                                let frames = remote_frames.clone();
                                let track_clone = track.clone();
                                tokio::spawn(async move {
                                    handle_remote_track(track_clone, frames).await;
                                });
                                new_tracks_found += 1;
                            }
                        }
                    }
                }

                // If we found tracks on this attempt and we've waited at least a bit, we're done
                if new_tracks_found > 0 && attempts > 0 {
                    tracing::info!("Found {} new tracks after {} attempts", new_tracks_found, attempts + 1);
                    break;
                }

                attempts += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            if attempts >= max_attempts && new_tracks_found == 0 {
                tracing::warn!("No new tracks found after {} attempts", max_attempts);
            }
        });

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

    /// Create a new offer for renegotiation (e.g., when adding screen share track)
    pub async fn create_offer(&self) -> Result<String> {
        let pc_guard = self.peer_connection.read().await;
        let pc = pc_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        let offer = pc.create_offer(None).await?;
        pc.set_local_description(offer.clone()).await?;

        tracing::info!("Created new offer for renegotiation");
        Ok(offer.sdp)
    }

    /// Send a video frame to the SFU
    /// The frame should be RGB data that will be encoded to H.264
    pub async fn send_frame(&self, frame: &VideoFrame) -> Result<()> {
        let track_guard = self.local_video_track.read().await;
        let track = track_guard
            .as_ref()
            .ok_or_else(|| anyhow!("No local track"))?;

        // Encode the RGB frame to H.264 using GStreamer (hardware accelerated)
        let encoded = {
            let mut encoder_guard = self.encoder.lock().map_err(|e| anyhow!("Encoder lock error: {}", e))?;

            // Create encoder on first frame (lazy initialization)
            // This allows us to get the actual frame dimensions
            if encoder_guard.is_none() {
                tracing::info!("Creating H.264 encoder for {}x{}", frame.width, frame.height);
                match GstVp8Encoder::new(frame.width, frame.height) {
                    Ok(enc) => {
                        *encoder_guard = Some(enc);
                    }
                    Err(e) => {
                        tracing::error!("Failed to create H.264 encoder: {}", e);
                        return Err(anyhow!("Failed to create H.264 encoder: {}", e));
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

    /// Get the latest frame from a remote user's track
    pub async fn get_remote_frame(&self, user_id: Uuid, track_type: TrackType) -> Option<RemoteVideoFrame> {
        self.remote_frames.read().await.get(&(user_id, track_type)).cloned()
    }

    /// Get the latest webcam frame from a remote user (convenience method)
    pub async fn get_remote_webcam_frame(&self, user_id: Uuid) -> Option<RemoteVideoFrame> {
        self.get_remote_frame(user_id, TrackType::Webcam).await
    }

    /// Get the latest screen frame from a remote user (convenience method)
    pub async fn get_remote_screen_frame(&self, user_id: Uuid) -> Option<RemoteVideoFrame> {
        self.get_remote_frame(user_id, TrackType::Screen).await
    }

    /// Get all remote user IDs with webcam frames
    pub async fn get_remote_users(&self) -> Vec<Uuid> {
        self.remote_frames
            .read()
            .await
            .keys()
            .filter(|(_, track_type)| *track_type == TrackType::Webcam)
            .map(|(user_id, _)| *user_id)
            .collect()
    }

    /// Get all remote user IDs with screen share frames
    pub async fn get_remote_screen_sharers(&self) -> Vec<Uuid> {
        self.remote_frames
            .read()
            .await
            .keys()
            .filter(|(_, track_type)| *track_type == TrackType::Screen)
            .map(|(user_id, _)| *user_id)
            .collect()
    }

    /// Start screen sharing
    /// Creates a screen share track and adds it to the peer connection
    /// Note: Caller needs to trigger renegotiation after this
    pub async fn start_screen_share(&self) -> Result<()> {
        let pc_guard = self.peer_connection.read().await;
        let pc = pc_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        // Check if screen track already exists
        if self.local_screen_track.read().await.is_some() {
            return Err(anyhow!("Screen share already active"));
        }

        // Create screen share track with unique stream ID and H.264 codec
        let screen_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/H264".to_string(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_string(),
                rtcp_feedback: vec![],
            },
            "screen".to_string(),
            "stream-local-screen".to_string(),
        ));

        // Add track to peer connection
        pc.add_track(screen_track.clone()).await?;
        *self.local_screen_track.write().await = Some(screen_track);

        tracing::info!("Screen share track created and added to peer connection");
        Ok(())
    }

    /// Stop screen sharing
    /// Note: Caller needs to trigger renegotiation after this
    pub async fn stop_screen_share(&self) -> Result<()> {
        // Remove track from peer connection
        if let Some(track) = self.local_screen_track.write().await.take() {
            // Note: webrtc-rs doesn't have a direct remove_track method
            // The track will be marked as ended and the server will handle cleanup
            // during renegotiation
            tracing::info!("Screen share track removed");

            // Clean up encoder
            if let Ok(mut encoder) = self.screen_encoder.lock() {
                *encoder = None;
            }
        }

        Ok(())
    }

    /// Check if screen sharing is active
    pub async fn is_screen_sharing(&self) -> bool {
        self.local_screen_track.read().await.is_some()
    }

    /// Send a screen frame to the SFU
    /// The frame should be RGB data that will be encoded to H.264 (hardware accelerated)
    pub async fn send_screen_frame(&self, frame: &VideoFrame, fps: u32) -> Result<()> {
        let track_guard = self.local_screen_track.read().await;
        let track = track_guard
            .as_ref()
            .ok_or_else(|| anyhow!("No screen share track - call start_screen_share first"))?;

        // Encode the RGB frame to H.264 using GStreamer hardware encoder
        let encoded = {
            let mut encoder_guard = self.screen_encoder.lock().map_err(|e| anyhow!("Screen encoder lock error: {}", e))?;

            // Create encoder on first frame (lazy initialization)
            if encoder_guard.is_none() {
                tracing::info!("Creating H.264 screen encoder for {}x{} @{}fps", frame.width, frame.height, fps);
                match GstScreenEncoder::new_with_fps(frame.width, frame.height, fps) {
                    Ok(enc) => {
                        *encoder_guard = Some(enc);
                    }
                    Err(e) => {
                        tracing::error!("Failed to create H.264 screen encoder: {}", e);
                        return Err(anyhow!("Failed to create H.264 screen encoder: {}", e));
                    }
                }
            }

            let encoder = encoder_guard.as_ref().unwrap();

            // Check if dimensions or fps changed, recreate encoder if needed
            if encoder.width() != frame.width || encoder.height() != frame.height || encoder.fps() != fps {
                tracing::info!("Screen size/fps changed to {}x{} @{}fps, recreating encoder",
                    frame.width, frame.height, fps);
                drop(encoder_guard);
                let mut encoder_guard = self.screen_encoder.lock().map_err(|e| anyhow!("Screen encoder lock error: {}", e))?;
                *encoder_guard = Some(GstScreenEncoder::new_with_fps(frame.width, frame.height, fps)?);
                encoder_guard.as_ref().unwrap().encode(frame)?
            } else {
                encoder.encode(frame)?
            }
        };

        // Skip if encoder returned empty (still buffering)
        if encoded.is_empty() {
            return Ok(());
        }

        // Create RTP sample with duration based on fps
        use webrtc::media::Sample;
        let duration_ms = 1000 / fps as u64;
        let sample = Sample {
            data: encoded.into(),
            duration: std::time::Duration::from_millis(duration_ms),
            ..Default::default()
        };

        track.write_sample(&sample).await?;
        Ok(())
    }

    /// Force the encoder to generate a keyframe (IDR frame)
    /// This is called when a new subscriber joins and needs to start decoding
    pub fn force_keyframe(&self, track_type: miscord_protocol::TrackType) -> Result<()> {
        match track_type {
            miscord_protocol::TrackType::Webcam => {
                if let Ok(encoder_guard) = self.encoder.lock() {
                    if let Some(encoder) = encoder_guard.as_ref() {
                        encoder.force_keyframe()?;
                        tracing::info!("Forced keyframe on webcam encoder");
                    } else {
                        tracing::debug!("No webcam encoder to force keyframe");
                    }
                }
            }
            miscord_protocol::TrackType::Screen => {
                if let Ok(encoder_guard) = self.screen_encoder.lock() {
                    if let Some(encoder) = encoder_guard.as_ref() {
                        encoder.force_keyframe()?;
                        tracing::info!("Forced keyframe on screen encoder");
                    } else {
                        tracing::debug!("No screen encoder to force keyframe");
                    }
                }
            }
        }
        Ok(())
    }

    /// Disconnect from the SFU
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(pc) = self.peer_connection.write().await.take() {
            pc.close().await?;
        }
        *self.local_video_track.write().await = None;
        *self.local_screen_track.write().await = None;
        *self.channel_id.write().await = None;
        *self.ice_candidate_tx.write().await = None;
        self.remote_frames.write().await.clear();
        self.handled_tracks.write().await.clear();

        // Clean up webcam encoder
        if let Ok(mut encoder) = self.encoder.lock() {
            *encoder = None;
        }

        // Clean up screen encoder
        if let Ok(mut encoder) = self.screen_encoder.lock() {
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
    remote_frames: Arc<RwLock<HashMap<(Uuid, TrackType), RemoteVideoFrame>>>,
) {
    let stream_id = track.stream_id().to_string();

    // Parse user ID and track type from stream ID
    // Format: "stream-{user_id}-{track_type}" (new) or "stream-{user_id}" (legacy)
    let (user_id, track_type) = match parse_stream_id(&stream_id) {
        Some(parsed) => parsed,
        None => {
            tracing::warn!("Invalid stream ID format: {}", stream_id);
            return;
        }
    };

    tracing::info!(
        "Receiving remote {:?} track from user {}, track: {}",
        track_type,
        user_id,
        track.id()
    );

    // Note: Removed the 100ms delay that was here - it added unnecessary latency.
    // The RTP read loop will naturally wait for packets to arrive.

    // Create H.264 decoder for this track (hardware accelerated)
    let decoder = match GstVp8Decoder::new() {
        Ok(dec) => dec,
        Err(e) => {
            tracing::error!("Failed to create H.264 decoder for user {}: {}", user_id, e);
            return;
        }
    };

    // Read RTP packets and decode to frames
    tracing::info!("Starting RTP read loop for user {} {:?}", user_id, track_type);
    let mut packet_count = 0u64;
    loop {
        match track.read_rtp().await {
            Ok((rtp_packet, _attributes)) => {
                packet_count += 1;
                if packet_count % 100 == 1 {
                    tracing::info!("Received RTP packet {} for user {} {:?}, payload size: {}",
                        packet_count, user_id, track_type, rtp_packet.payload.len());
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

                // Decode H.264 packet to frame using GStreamer hardware decoder
                match decoder.decode(&rtp_bytes) {
                    Ok(Some(frame)) => {
                        // Only log every 100th decoded frame to reduce spam
                        if packet_count % 100 == 1 {
                            tracing::info!("Decoded {:?} frame for user {}: {}x{}", track_type, user_id, frame.width, frame.height);
                        }
                        let remote_frame = RemoteVideoFrame {
                            user_id,
                            track_type,
                            width: frame.width,
                            height: frame.height,
                            data: frame.data,
                        };
                        remote_frames.write().await.insert((user_id, track_type), remote_frame);
                    }
                    Ok(None) => {
                        // Decoder still buffering, no frame yet
                    }
                    Err(e) => {
                        tracing::warn!("H.264 decode error for user {} {:?}: {}", user_id, track_type, e);
                    }
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Track closed - normal shutdown
                if error_msg.contains("closed") {
                    tracing::info!("Remote {:?} track closed for user {}", track_type, user_id);
                    break;
                }
                // RTPReceiver gone - track was removed (e.g., screen share stopped)
                if error_msg.contains("RTPReceiver must not be nil") {
                    tracing::info!("Remote {:?} track ended for user {} (RTPReceiver removed)", track_type, user_id);
                    break;
                }
                tracing::warn!("Error reading RTP from remote track: {}", e);
            }
        }
    }

    // Remove from remote frames when track ends
    remote_frames.write().await.remove(&(user_id, track_type));
}

/// Parse stream ID to extract user ID and track type
/// Supports both new format "stream-{user_id}-{track_type}" and legacy "stream-{user_id}"
fn parse_stream_id(stream_id: &str) -> Option<(Uuid, TrackType)> {
    let stripped = stream_id.strip_prefix("stream-")?;

    // Try new format first: "stream-{uuid}-{webcam|screen}"
    if let Some(pos) = stripped.rfind('-') {
        let (uuid_part, type_part) = stripped.split_at(pos);
        let type_part = &type_part[1..]; // Skip the '-'

        // Check if this is a valid track type suffix
        let track_type = match type_part.to_lowercase().as_str() {
            "webcam" => Some(TrackType::Webcam),
            "screen" => Some(TrackType::Screen),
            _ => None,
        };

        if let Some(tt) = track_type {
            if let Ok(uuid) = Uuid::parse_str(uuid_part) {
                return Some((uuid, tt));
            }
        }
    }

    // Fall back to legacy format: "stream-{uuid}" (assume webcam)
    if let Ok(uuid) = Uuid::parse_str(stripped) {
        return Some((uuid, TrackType::Webcam));
    }

    None
}

