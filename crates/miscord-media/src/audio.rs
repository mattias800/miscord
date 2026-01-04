use anyhow::Result;
use opus::{Decoder as OpusDecoder, Encoder as OpusEncoder};

/// Audio sample rate used throughout the application (48kHz)
pub const SAMPLE_RATE: u32 = 48000;

/// Audio channels (stereo)
pub const CHANNELS: usize = 2;

/// Frame size in samples (20ms at 48kHz = 960 samples)
pub const FRAME_SIZE: usize = 960;

/// Opus audio encoder
pub struct AudioEncoder {
    encoder: OpusEncoder,
}

impl AudioEncoder {
    pub fn new() -> Result<Self> {
        let encoder = OpusEncoder::new(
            SAMPLE_RATE,
            opus::Channels::Stereo,
            opus::Application::Voip,
        )?;

        Ok(Self { encoder })
    }

    /// Encode PCM audio samples to Opus
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>> {
        let mut output = vec![0u8; 4000]; // Max opus packet size
        let len = self.encoder.encode(pcm, &mut output)?;
        output.truncate(len);
        Ok(output)
    }

    /// Encode float PCM audio samples to Opus
    pub fn encode_float(&mut self, pcm: &[f32]) -> Result<Vec<u8>> {
        let mut output = vec![0u8; 4000];
        let len = self.encoder.encode_float(pcm, &mut output)?;
        output.truncate(len);
        Ok(output)
    }

    /// Set the bitrate (in bits per second)
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        self.encoder.set_bitrate(opus::Bitrate::Bits(bitrate as i32))?;
        Ok(())
    }
}

impl Default for AudioEncoder {
    fn default() -> Self {
        Self::new().expect("Failed to create audio encoder")
    }
}

/// Opus audio decoder
pub struct AudioDecoder {
    decoder: OpusDecoder,
}

impl AudioDecoder {
    pub fn new() -> Result<Self> {
        let decoder = OpusDecoder::new(SAMPLE_RATE, opus::Channels::Stereo)?;

        Ok(Self { decoder })
    }

    /// Decode Opus to PCM audio samples
    pub fn decode(&mut self, opus_data: &[u8]) -> Result<Vec<i16>> {
        let mut output = vec![0i16; FRAME_SIZE * CHANNELS];
        let len = self.decoder.decode(opus_data, &mut output, false)?;
        output.truncate(len * CHANNELS);
        Ok(output)
    }

    /// Decode Opus to float PCM audio samples
    pub fn decode_float(&mut self, opus_data: &[u8]) -> Result<Vec<f32>> {
        let mut output = vec![0f32; FRAME_SIZE * CHANNELS];
        let len = self.decoder.decode_float(opus_data, &mut output, false)?;
        output.truncate(len * CHANNELS);
        Ok(output)
    }

    /// Handle packet loss by generating concealment audio
    pub fn decode_loss(&mut self) -> Result<Vec<i16>> {
        let mut output = vec![0i16; FRAME_SIZE * CHANNELS];
        let len = self.decoder.decode(&[], &mut output, true)?;
        output.truncate(len * CHANNELS);
        Ok(output)
    }
}

impl Default for AudioDecoder {
    fn default() -> Self {
        Self::new().expect("Failed to create audio decoder")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let mut encoder = AudioEncoder::new().unwrap();
        let mut decoder = AudioDecoder::new().unwrap();

        // Create test audio (silence)
        let pcm: Vec<i16> = vec![0; FRAME_SIZE * CHANNELS];

        // Encode
        let encoded = encoder.encode(&pcm).unwrap();
        assert!(!encoded.is_empty());

        // Decode
        let decoded = decoder.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), FRAME_SIZE * CHANNELS);
    }
}
