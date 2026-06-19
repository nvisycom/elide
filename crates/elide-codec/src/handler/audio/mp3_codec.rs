//! MP3 decode/encode helpers: `symphonia` to interleaved f32 PCM,
//! `mp3lame-encoder` back to MP3.

use std::io::{Cursor, ErrorKind as IoErrorKind};

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use mp3lame_encoder::{Builder, FlushNoGap, InterleavedPcm, MonoPcm};
use symphonia::core::audio::conv::{ConvertibleSample, FromSample};
use symphonia::core::audio::{Audio as _, AudioBuffer, GenericAudioBufferRef};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::{get_codecs, get_probe};

/// Decoded MP3 as interleaved f32 PCM, with the parameters needed to
/// re-encode.
pub(super) struct DecodedMp3 {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Probe the channel count of the first audio track without decoding.
///
/// The loader uses this to reject >2-channel inputs before they reach
/// the redact path: LAME encodes only mono and stereo, and silently
/// downmixing would edit the unredacted audio.
pub(super) fn probe_channels(bytes: &Bytes) -> Result<u16> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.clone())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let reader = get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| Error::new(ErrorKind::Validation, format!("MP3 probe failed: {e}")))?;

    let track = reader
        .default_track(TrackType::Audio)
        .ok_or_else(|| Error::new(ErrorKind::Validation, "MP3 stream has no audio track"))?;

    let channels = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .and_then(|a| a.channels.as_ref())
        .map(|c| c.count())
        .ok_or_else(|| Error::new(ErrorKind::Validation, "MP3 track is missing channel info"))?;

    u16::try_from(channels)
        .map_err(|_| Error::new(ErrorKind::Validation, "MP3 channel count exceeds u16"))
}

/// Decode the whole MP3 to interleaved f32 PCM.
pub(super) fn decode_to_pcm(bytes: &Bytes) -> Result<DecodedMp3> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.clone())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let mut reader = get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| Error::new(ErrorKind::Validation, format!("MP3 probe failed: {e}")))?;

    let track = reader
        .default_track(TrackType::Audio)
        .ok_or_else(|| Error::new(ErrorKind::Validation, "MP3 stream has no audio track"))?;
    let track_id = track.id;

    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| Error::new(ErrorKind::Validation, "MP3 track is missing audio params"))?
        .clone();

    let sample_rate = audio_params
        .sample_rate
        .ok_or_else(|| Error::new(ErrorKind::Validation, "MP3 track is missing a sample rate"))?;
    let channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .ok_or_else(|| Error::new(ErrorKind::Validation, "MP3 track is missing channel info"))?;
    let channels_u16 = u16::try_from(channels)
        .map_err(|_| Error::new(ErrorKind::Validation, "MP3 channel count exceeds u16"))?;

    let mut decoder = get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| {
            Error::new(
                ErrorKind::Validation,
                format!("MP3 decoder init failed: {e}"),
            )
        })?;

    let mut samples = Vec::<f32>::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(io_err)) if io_err.kind() == IoErrorKind::UnexpectedEof => break,
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Validation,
                    format!("MP3 packet read failed: {e}"),
                ));
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf_ref) => append_interleaved_f32(&buf_ref, channels, &mut samples),
            // A single corrupt frame is skipped rather than aborting,
            // matching symphonia's reference player.
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Validation,
                    format!("MP3 decode failed: {e}"),
                ));
            }
        }
    }

    Ok(DecodedMp3 {
        samples,
        sample_rate,
        channels: channels_u16,
    })
}

/// Interleave one decoded packet's planar channels into `out`.
fn append_interleaved_f32(
    buf_ref: &GenericAudioBufferRef<'_>,
    channels: usize,
    out: &mut Vec<f32>,
) {
    fn extend<S: ConvertibleSample + Copy>(
        buf: &AudioBuffer<S>,
        channels: usize,
        out: &mut Vec<f32>,
    ) where
        f32: FromSample<S>,
    {
        let frames = buf.frames();
        out.reserve(frames * channels);
        for frame in 0..frames {
            for ch in 0..channels {
                let plane = buf.plane(ch).expect("plane for known channel index");
                out.push(<f32 as FromSample<S>>::from_sample(plane[frame]));
            }
        }
    }

    match buf_ref {
        GenericAudioBufferRef::U8(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::U16(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::U32(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::S8(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::S16(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::S32(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::F32(buf) => extend(buf, channels, out),
        GenericAudioBufferRef::F64(buf) => extend(buf, channels, out),
        // MP3 decoders never emit 24-bit buffers; handle explicitly so a
        // future symphonia change doesn't slip through untested.
        GenericAudioBufferRef::U24(_) | GenericAudioBufferRef::S24(_) => {
            unreachable!("MP3 decoder does not emit 24-bit sample buffers");
        }
    }
}

/// Encode interleaved f32 PCM back to MP3 at `bitrate_bps`.
pub(super) fn encode_from_pcm(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    bitrate_bps: u32,
) -> Result<Vec<u8>> {
    let bitrate = snap_bitrate(bitrate_bps);

    let mut builder =
        Builder::new().ok_or_else(|| Error::new(ErrorKind::Redaction, "LAME builder failed"))?;
    builder
        .set_sample_rate(sample_rate)
        .map_err(|e| Error::new(ErrorKind::Validation, format!("LAME sample-rate: {e:?}")))?;
    builder
        .set_num_channels(channels as u8)
        .map_err(|e| Error::new(ErrorKind::Validation, format!("LAME channels: {e:?}")))?;
    builder
        .set_brate(bitrate)
        .map_err(|e| Error::new(ErrorKind::Validation, format!("LAME bitrate: {e:?}")))?;
    builder
        .set_quality(mp3lame_encoder::Quality::Good)
        .map_err(|e| Error::new(ErrorKind::Validation, format!("LAME quality: {e:?}")))?;

    let mut encoder = builder
        .build()
        .map_err(|e| Error::new(ErrorKind::Redaction, format!("LAME init failed: {e:?}")))?;

    let frames = samples.len() / channels.max(1) as usize;
    let mut out = Vec::<u8>::with_capacity(mp3lame_encoder::max_required_buffer_size(frames));
    match channels {
        1 => encoder
            .encode_to_vec(MonoPcm(samples), &mut out)
            .map_err(|e| Error::new(ErrorKind::Redaction, format!("LAME encode: {e:?}")))?,
        2 => encoder
            .encode_to_vec(InterleavedPcm(samples), &mut out)
            .map_err(|e| Error::new(ErrorKind::Redaction, format!("LAME encode: {e:?}")))?,
        n => {
            return Err(Error::new(
                ErrorKind::Validation,
                format!("LAME supports 1 or 2 channels, got {n}"),
            ));
        }
    };
    encoder
        .flush_to_vec::<FlushNoGap>(&mut out)
        .map_err(|e| Error::new(ErrorKind::Redaction, format!("LAME flush: {e:?}")))?;
    Ok(out)
}

/// Duration of an MP3 clip in milliseconds.
///
/// Prefers the container-level duration from
/// [`probe_duration_ms`](super::duration::probe_duration_ms); a raw LAME
/// stream has no Xing/Info header and reports no duration, so this falls
/// back to decoding the clip and dividing the per-channel frame count by
/// the sample rate.
pub(super) fn duration_ms(bytes: &Bytes) -> Result<u64> {
    if let Ok(ms) = super::duration::probe_duration_ms(bytes, "mp3") {
        return Ok(ms);
    }
    let decoded = decode_to_pcm(bytes)?;
    let frames = decoded.samples.len() as u64 / decoded.channels.max(1) as u64;
    Ok(frames * 1_000 / decoded.sample_rate.max(1) as u64)
}

/// Average bitrate (bits/sec) of a clip from its byte size and duration.
/// Falls back to 128 kbps for a zero-length clip.
pub(super) fn average_bitrate_bps(file_bytes: usize, duration_ms: u64) -> u32 {
    if duration_ms == 0 {
        return 128_000;
    }
    ((file_bytes as u64 * 8 * 1_000) / duration_ms) as u32
}

/// Snap an arbitrary bits-per-second target to the nearest
/// [`mp3lame_encoder::Bitrate`] LAME accepts.
fn snap_bitrate(bps: u32) -> mp3lame_encoder::Bitrate {
    use mp3lame_encoder::Bitrate::*;

    const ALL: &[mp3lame_encoder::Bitrate] = &[
        Kbps8, Kbps16, Kbps24, Kbps32, Kbps40, Kbps48, Kbps64, Kbps80, Kbps96, Kbps112, Kbps128,
        Kbps160, Kbps192, Kbps224, Kbps256, Kbps320,
    ];
    let kbps = (bps + 500) / 1_000;
    ALL.iter()
        .copied()
        .min_by_key(|b| ((*b as u32) as i64 - kbps as i64).abs())
        .unwrap_or(Kbps128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_bitrate_picks_nearest() {
        use mp3lame_encoder::Bitrate;
        assert_eq!(snap_bitrate(127_000) as u32, Bitrate::Kbps128 as u32);
        assert_eq!(snap_bitrate(96_000) as u32, Bitrate::Kbps96 as u32);
        assert_eq!(snap_bitrate(1_000_000) as u32, Bitrate::Kbps320 as u32);
    }

    #[test]
    fn round_trips_silence() {
        // 0.5s of mono silence at 16 kHz: encode, decode, compare length.
        let samples = vec![0f32; 8_000];
        let encoded = encode_from_pcm(&samples, 16_000, 1, 64_000).unwrap();
        assert!(!encoded.is_empty());
        let decoded = decode_to_pcm(&Bytes::from(encoded)).unwrap();
        assert_eq!(decoded.sample_rate, 16_000);
        assert_eq!(decoded.channels, 1);
        // LAME adds encoder delay/padding, so allow a couple frames of drift.
        let diff = (decoded.samples.len() as i64 - samples.len() as i64).abs();
        assert!(diff < 2_400, "round-trip drift too large: {diff}");
    }
}
