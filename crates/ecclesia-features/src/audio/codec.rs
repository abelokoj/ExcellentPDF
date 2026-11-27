//! Pure-Rust WAV (16-bit PCM) and Ogg/Opus container encoding, decoding, and `.expdf` bundling (plan L.1).

use std::time::Duration;

use ecclesia_core::notebook::{AudioFormat, AudioRecordingMeta, Notebook, Result};

use crate::audio::device::{AudioError, AudioResult};

/// Decoded 16-bit PCM WAV data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavData {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (`1` = mono, `2` = stereo).
    pub channels: u16,
    /// Interleaved 16-bit signed PCM samples.
    pub samples: Vec<i16>,
}

impl WavData {
    /// Duration of this PCM clip.
    #[must_use]
    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 || self.channels == 0 {
            return Duration::ZERO;
        }
        let frames = self.samples.len() / usize::from(self.channels);
        Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate))
    }
}

/// Encodes interleaved 16-bit signed PCM samples into a standard RIFF/WAVE file.
///
/// # Errors
///
/// Returns [`AudioError::InvalidAudio`] if `sample_rate` or `channels` is zero.
pub fn encode_wav(samples: &[i16], sample_rate: u32, channels: u16) -> AudioResult<Vec<u8>> {
    if sample_rate == 0 || channels == 0 {
        return Err(AudioError::InvalidAudio(
            "sample_rate and channels must be positive".into(),
        ));
    }
    let bits_per_sample: u16 = 16;
    let block_align = channels
        .checked_mul(bits_per_sample / 8)
        .ok_or_else(|| AudioError::InvalidAudio("channel count overflow".into()))?;
    let byte_rate = sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| AudioError::InvalidAudio("byte rate overflow".into()))?;
    let data_bytes = u32::try_from(samples.len() * 2)
        .map_err(|_| AudioError::InvalidAudio("wav data exceeds 4 GiB limit".into()))?;
    let riff_size = 36u32
        .checked_add(data_bytes)
        .ok_or_else(|| AudioError::InvalidAudio("wav file size overflow".into()))?;

    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    // fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(out)
}

/// Decodes a standard 16-bit PCM RIFF/WAVE file.
///
/// # Errors
///
/// Returns [`AudioError::InvalidAudio`] if the WAV header or chunks are malformed or unsupported.
pub fn decode_wav(bytes: &[u8]) -> AudioResult<WavData> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AudioError::InvalidAudio(
            "not a valid RIFF/WAVE file".into(),
        ));
    }
    let mut pos = 12usize;
    let mut sample_rate = None;
    let mut channels = None;
    let mut samples = None;

    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        pos += 8;
        if pos + chunk_len > bytes.len() {
            return Err(AudioError::InvalidAudio("truncated WAV chunk".into()));
        }
        let body = &bytes[pos..pos + chunk_len];
        if chunk_id == b"fmt " {
            if body.len() < 16 {
                return Err(AudioError::InvalidAudio("short WAV fmt chunk".into()));
            }
            let audio_format = u16::from_le_bytes([body[0], body[1]]);
            let ch = u16::from_le_bytes([body[2], body[3]]);
            let sr = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            let bps = u16::from_le_bytes([body[14], body[15]]);
            if audio_format != 1 || bps != 16 {
                return Err(AudioError::InvalidAudio(
                    "only 16-bit PCM WAV is supported".into(),
                ));
            }
            channels = Some(ch);
            sample_rate = Some(sr);
        } else if chunk_id == b"data" {
            let mut decoded = Vec::with_capacity(body.len() / 2);
            for &[lo, hi] in body.as_chunks::<2>().0 {
                decoded.push(i16::from_le_bytes([lo, hi]));
            }
            samples = Some(decoded);
        }
        pos += chunk_len + (chunk_len % 2);
    }

    Ok(WavData {
        sample_rate: sample_rate
            .ok_or_else(|| AudioError::InvalidAudio("WAV missing fmt chunk".into()))?,
        channels: channels
            .ok_or_else(|| AudioError::InvalidAudio("WAV missing channel count".into()))?,
        samples: samples
            .ok_or_else(|| AudioError::InvalidAudio("WAV missing data chunk".into()))?,
    })
}

/// An encoded Opus packet paired with its duration in 48 kHz samples (RFC 7845).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggOpusPacket {
    /// Raw Opus packet bytes (typically 20 ms = 960 samples at 48 kHz).
    pub payload: Vec<u8>,
    /// Number of 48 kHz PCM samples represented by this packet.
    pub granule_samples: u64,
}

/// Decoded Ogg/Opus container metadata and packets (RFC 7845).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggOpusStream {
    /// Original input sample rate recorded in `OpusHead`.
    pub input_sample_rate: u32,
    /// Channel count (`1` or `2`).
    pub channels: u8,
    /// Total duration derived from the final 48 kHz granule position.
    pub duration: Duration,
    /// Vendor string from `OpusTags`.
    pub vendor: String,
    /// Encoded Opus packets.
    pub packets: Vec<OggOpusPacket>,
}

/// Muxes Opus packets into a standards-compliant Ogg/Opus container (`OpusHead` + `OpusTags` +
/// CRC-verified `OggS` pages, RFC 7845).
///
/// # Errors
///
/// Returns [`AudioError::InvalidAudio`] if `channels` is zero.
pub fn encode_ogg_opus(
    packets: &[OggOpusPacket],
    input_sample_rate: u32,
    channels: u8,
    vendor: &str,
) -> AudioResult<Vec<u8>> {
    if channels == 0 {
        return Err(AudioError::InvalidAudio("channels must be positive".into()));
    }
    let serial: u32 = 0x4558_5044; // "EXPD"
    let mut out = Vec::new();
    let mut seq: u32 = 0;

    // Page 0: OpusHead (19 bytes, BOS flag 0x02)
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1); // version
    head.push(channels);
    head.extend_from_slice(&312u16.to_le_bytes()); // pre-skip (typical 6.5 ms at 48kHz)
    head.extend_from_slice(&input_sample_rate.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes()); // output gain
    head.push(0); // channel mapping family 0
    write_ogg_page(&mut out, 0x02, 0, serial, seq, &head);
    seq += 1;

    // Page 1: OpusTags
    let vendor_bytes = vendor.as_bytes();
    let mut tags = Vec::with_capacity(16 + vendor_bytes.len());
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(vendor_bytes.len() as u32).to_le_bytes());
    tags.extend_from_slice(vendor_bytes);
    tags.extend_from_slice(&0u32.to_le_bytes()); // user comment list length
    write_ogg_page(&mut out, 0x00, 0, serial, seq, &tags);
    seq += 1;

    // Audio packet pages
    let mut granule: u64 = 0;
    for (idx, pkt) in packets.iter().enumerate() {
        granule = granule.saturating_add(pkt.granule_samples);
        let flags = if idx + 1 == packets.len() { 0x04 } else { 0x00 }; // EOS on last page
        write_ogg_page(&mut out, flags, granule, serial, seq, &pkt.payload);
        seq += 1;
    }

    Ok(out)
}

/// Demuxes and validates an Ogg/Opus container, checking `OggS` sync headers, CRC-32 checksums,
/// `OpusHead`, `OpusTags`, and 48 kHz granule timestamps.
///
/// # Errors
///
/// Returns [`AudioError::InvalidAudio`] if any page magic, checksum, or header is invalid.
pub fn decode_ogg_opus(bytes: &[u8]) -> AudioResult<OggOpusStream> {
    let mut pos = 0usize;
    let mut input_sample_rate = 48_000u32;
    let mut channels = 1u8;
    let mut vendor = String::new();
    let mut packets = Vec::new();
    let mut prev_granule = 0u64;
    let mut saw_head = false;
    let mut saw_tags = false;

    while pos < bytes.len() {
        if pos + 27 > bytes.len() || &bytes[pos..pos + 4] != b"OggS" {
            return Err(AudioError::InvalidAudio("invalid OggS page header".into()));
        }
        let granule = u64::from_le_bytes([
            bytes[pos + 6],
            bytes[pos + 7],
            bytes[pos + 8],
            bytes[pos + 9],
            bytes[pos + 10],
            bytes[pos + 11],
            bytes[pos + 12],
            bytes[pos + 13],
        ]);
        let stored_crc = u32::from_le_bytes([
            bytes[pos + 22],
            bytes[pos + 23],
            bytes[pos + 24],
            bytes[pos + 25],
        ]);
        let seg_count = usize::from(bytes[pos + 26]);
        let header_len = 27 + seg_count;
        if pos + header_len > bytes.len() {
            return Err(AudioError::InvalidAudio(
                "truncated Ogg segment table".into(),
            ));
        }
        let body_len: usize = bytes[pos + 27..pos + header_len]
            .iter()
            .map(|&b| usize::from(b))
            .sum();
        let page_end = pos + header_len + body_len;
        if page_end > bytes.len() {
            return Err(AudioError::InvalidAudio("truncated Ogg page body".into()));
        }

        // Verify Ogg CRC-32 with checksum field zeroed.
        let mut crc_buf = bytes[pos..page_end].to_vec();
        crc_buf[22..26].fill(0);
        if ogg_crc32(&crc_buf) != stored_crc {
            return Err(AudioError::InvalidAudio("Ogg page CRC mismatch".into()));
        }

        let body = &bytes[pos + header_len..page_end];
        if !saw_head {
            if body.len() < 19 || &body[0..8] != b"OpusHead" {
                return Err(AudioError::InvalidAudio("missing OpusHead packet".into()));
            }
            channels = body[9];
            input_sample_rate = u32::from_le_bytes([body[12], body[13], body[14], body[15]]);
            saw_head = true;
        } else if !saw_tags {
            if body.len() < 16 || &body[0..8] != b"OpusTags" {
                return Err(AudioError::InvalidAudio("missing OpusTags packet".into()));
            }
            let vlen = u32::from_le_bytes([body[8], body[9], body[10], body[11]]) as usize;
            if 12 + vlen <= body.len() {
                vendor = String::from_utf8_lossy(&body[12..12 + vlen]).into_owned();
            }
            saw_tags = true;
        } else {
            let delta = granule.saturating_sub(prev_granule);
            prev_granule = granule;
            packets.push(OggOpusPacket {
                payload: body.to_vec(),
                granule_samples: delta,
            });
        }

        pos = page_end;
    }

    if !saw_head || !saw_tags {
        return Err(AudioError::InvalidAudio(
            "incomplete Ogg/Opus stream headers".into(),
        ));
    }

    let duration = Duration::from_secs_f64(prev_granule as f64 / 48_000.0);
    Ok(OggOpusStream {
        input_sample_rate,
        channels,
        duration,
        vendor,
        packets,
    })
}

/// Parameters for bundling an encoded audio recording into a [`Notebook`] (plan L.1).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBundleSpec {
    /// Stable recording UUID.
    pub recording_id: String,
    /// Audio container/codec format.
    pub format: AudioFormat,
    /// Encoded WAV or Ogg/Opus bytes.
    pub bytes: Vec<u8>,
    /// Recording start timestamp in milliseconds on the ink clock.
    pub started_at_ms: f64,
    /// Total duration in milliseconds.
    pub duration_ms: f64,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (`1` = mono, `2` = stereo).
    pub channels: u16,
    /// Optional lecture or recording title.
    pub title: Option<String>,
}

/// Bundles an encoded audio recording into a [`Notebook`]'s `assets/audio/` directory and records
/// its metadata entry in `notebook.json` (plan L.1 step 1).
///
/// # Errors
///
/// Returns an error if the asset path is invalid.
pub fn bundle_audio_in_notebook(
    notebook: &mut Notebook,
    spec: AudioBundleSpec,
) -> Result<AudioRecordingMeta> {
    let asset_path = format!(
        "assets/audio/{}.{}",
        spec.recording_id,
        spec.format.extension()
    );
    notebook.add_asset(&asset_path, spec.bytes)?;
    let meta = AudioRecordingMeta {
        id: spec.recording_id,
        asset_path,
        format: spec.format,
        started_at_ms: spec.started_at_ms,
        duration_ms: spec.duration_ms,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        title: spec.title,
    };
    notebook.metadata.audio.push(meta.clone());
    Ok(meta)
}

fn write_ogg_page(
    out: &mut Vec<u8>,
    header_type: u8,
    granule: u64,
    serial: u32,
    seq: u32,
    payload: &[u8],
) {
    let mut lacing = Vec::new();
    let mut remaining = payload.len();
    while remaining >= 255 {
        lacing.push(255u8);
        remaining -= 255;
    }
    lacing.push(remaining as u8);

    let start = out.len();
    out.extend_from_slice(b"OggS");
    out.push(0); // stream structure version
    out.push(header_type);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&serial.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    let crc_offset = out.len();
    out.extend_from_slice(&0u32.to_le_bytes()); // CRC placeholder
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    out.extend_from_slice(payload);

    let crc = ogg_crc32(&out[start..]);
    out[crc_offset..crc_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

/// Standard Ogg non-reflected CRC-32 (polynomial `0x04C11DB7`).
fn ogg_crc32(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            if (crc & 0x8000_0000) != 0 {
                crc = (crc << 1) ^ 0x04C1_1DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::notebook::{Orientation, PageSize, PaperTemplateId};

    #[test]
    fn wav_and_ogg_opus_round_trip_and_bundle_in_expdf() {
        // 1 second of 48 kHz mono sine-ish samples.
        let pcm: Vec<i16> = (0..48_000).map(|i| ((i % 100) as i16) * 200).collect();
        let wav_bytes = encode_wav(&pcm, 48_000, 1).expect("encode wav");
        let decoded = decode_wav(&wav_bytes).expect("decode wav");
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples, pcm);
        assert!((decoded.duration().as_secs_f64() - 1.0).abs() < 1e-4);

        // Ogg/Opus container with 50 packets * 960 samples = 48000 samples = 1.0 s.
        let packets: Vec<OggOpusPacket> = (0..50)
            .map(|_| OggOpusPacket {
                payload: vec![0xFC, 0xFF, 0xFE], // minimal valid Opus silence TOC frame
                granule_samples: 960,
            })
            .collect();
        let opus_bytes = encode_ogg_opus(&packets, 48_000, 1, "EcclesiaPDF").expect("encode opus");
        let stream = decode_ogg_opus(&opus_bytes).expect("decode opus");
        assert_eq!(stream.channels, 1);
        assert_eq!(stream.vendor, "EcclesiaPDF");
        assert_eq!(stream.packets.len(), 50);
        assert!((stream.duration.as_secs_f64() - 1.0).abs() < 1e-4);

        // Bundle inside .expdf and round-trip via ZIP.
        let mut nb = Notebook::new_blank(
            "Lecture",
            PageSize::A4,
            Orientation::Portrait,
            PaperTemplateId::Cornell,
        )
        .expect("nb");
        let meta = bundle_audio_in_notebook(
            &mut nb,
            AudioBundleSpec {
                recording_id: "lec-01".into(),
                format: AudioFormat::OggOpus,
                bytes: opus_bytes.clone(),
                started_at_ms: 10_000.0,
                duration_ms: 1_000.0,
                sample_rate: 48_000,
                channels: 1,
                title: Some("Intro".into()),
            },
        )
        .expect("bundle");
        let zip_bytes = nb.to_zip_bytes().expect("zip");
        let reopened = Notebook::from_zip_bytes(&zip_bytes).expect("reopen");
        assert_eq!(reopened.metadata.audio.len(), 1);
        assert_eq!(
            reopened.asset(&meta.asset_path),
            Some(opus_bytes.as_slice())
        );
    }
}
