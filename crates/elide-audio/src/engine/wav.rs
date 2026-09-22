//! WAV sample codec: `hound` reads samples, the shared redact pass mutates
//! the buffer, and `hound` re-writes them at the source clip's spec.

use std::io::Cursor;
use std::result;

use bytes::Bytes;
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use super::redact;
use crate::modality::Audio;

/// Validate that `bytes` parse as WAV.
///
/// Called on open so a malformed clip fails up front, not at the first
/// redaction.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the bytes are not a readable WAV.
pub(crate) fn validate(bytes: &Bytes) -> Result<()> {
    WavReader::new(Cursor::new(bytes.clone()))
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("not a valid WAV: {e}")))?;
    Ok(())
}

/// Redact `redactions` over the WAV clip `source`, returning fresh WAV bytes.
///
/// Reads the clip's [`WavSpec`], decodes every sample as the matching native
/// type, applies the batch, and re-encodes at the same spec.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] for an unreadable clip or an unsupported
/// sample format, or [`ErrorKind::Processing`] if re-encoding fails.
pub(crate) fn redact_all(source: &Bytes, redactions: &Redactions<Audio>) -> Result<Bytes> {
    // Sort by position so the per-type pass can walk the batch in a
    // deterministic order (and reverse it to apply right-to-left).
    let mut sorted = redactions.clone();
    sorted.sort_by_position();
    let spec = wav_spec(source)?;
    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 8) => redact_typed::<i8>(source, spec, &sorted),
        (SampleFormat::Int, 16) => redact_typed::<i16>(source, spec, &sorted),
        (SampleFormat::Int, 24 | 32) => redact_typed::<i32>(source, spec, &sorted),
        (SampleFormat::Float, 32) => redact_typed::<f32>(source, spec, &sorted),
        (fmt, bits) => Err(Error::new(
            ErrorKind::MalformedInput,
            format!("unsupported WAV sample format {fmt:?} at {bits} bits"),
        )),
    }
}

/// Decode every sample of `source` as `S`, apply the batch on the buffer,
/// then re-encode to fresh bytes at `spec`.
fn redact_typed<S>(source: &Bytes, spec: WavSpec, redactions: &Redactions<Audio>) -> Result<Bytes>
where
    S: hound::Sample + Default + Clone + redact::ToneSample,
{
    let mut reader = WavReader::new(Cursor::new(source.clone()))
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("WAV read failed: {e}")))?;
    let mut samples: Vec<S> = reader
        .samples::<S>()
        .collect::<result::Result<_, _>>()
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("WAV decode failed: {e}")))?;

    // The peak a `Tone` scales to. For integer WAV the container's range is
    // set by the bit depth, not the decode type: a 24-bit clip decodes into
    // `i32` but a sample must stay within `2^23 - 1`, so scale to that, not
    // `i32::MAX`. Float samples use unit scale.
    let full_scale = match spec.sample_format {
        SampleFormat::Int => (1u64 << (spec.bits_per_sample - 1)) as f32 - 1.0,
        SampleFormat::Float => 1.0,
    };

    // Walk the position-sorted batch in reverse, applying right-to-left so a
    // `Removed` span doesn't shift the sample indices of spans not yet
    // applied.
    for (location, replacement) in redactions.iter().rev() {
        redact::apply(
            &mut samples,
            location.span,
            replacement,
            spec.sample_rate,
            spec.channels,
            full_scale,
        );
    }

    let mut buf = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(&mut buf, spec)
            .map_err(|e| Error::new(ErrorKind::Processing, format!("WAV write failed: {e}")))?;
        for sample in samples {
            writer.write_sample(sample).map_err(|e| {
                Error::new(ErrorKind::Processing, format!("WAV encode failed: {e}"))
            })?;
        }
        writer
            .finalize()
            .map_err(|e| Error::new(ErrorKind::Processing, format!("WAV finalize failed: {e}")))?;
    }
    Ok(Bytes::from(buf.into_inner()))
}

/// Read the [`WavSpec`] (sample rate, channels, bit depth, format) from the
/// encoded bytes.
fn wav_spec(bytes: &Bytes) -> Result<WavSpec> {
    let reader = WavReader::new(Cursor::new(bytes.clone()))
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("not a valid WAV: {e}")))?;
    Ok(reader.spec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modality::{AudioLocation, AudioReplacement, Waveform};

    /// A `secs`-second 8 kHz mono `bits`-bit integer WAV of silence.
    fn silent_wav(bits: u16, secs: u32) -> Bytes {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: bits,
            sample_format: SampleFormat::Int,
        };
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = WavWriter::new(&mut buf, spec).unwrap();
            for _ in 0..(8_000 * secs) {
                w.write_sample(0i32).unwrap();
            }
            w.finalize().unwrap();
        }
        Bytes::from(buf.into_inner())
    }

    /// A `Tone` on a 24-bit clip must scale to the 24-bit range, not the
    /// `i32` the samples decode into: an `i32::MAX`-scaled sample overflows
    /// what `hound` can write as 24-bit and corrupts the clip.
    #[test]
    fn tone_stays_within_24_bit_range() {
        let mut batch: Redactions<Audio> = Redactions::new();
        batch.push(
            AudioLocation::from_millis(0, 100),
            AudioReplacement::Tone {
                hz: 440.0,
                amplitude: 1.0,
                waveform: Waveform::Sine,
            },
        );

        let out = redact_all(&silent_wav(24, 1), &batch).expect("24-bit tone re-encodes");

        // The clip still decodes, and every sample fits the 24-bit signed range.
        let mut reader = WavReader::new(Cursor::new(out)).unwrap();
        let peak = (1i32 << 23) - 1;
        assert!(
            reader
                .samples::<i32>()
                .map(|s| s.unwrap())
                .all(|s| s.abs() <= peak),
            "a sample exceeded the 24-bit range"
        );
    }
}
