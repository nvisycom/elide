//! Clip-duration probe via `symphonia`.
//!
//! Both audio handlers report a single full-clip chunk, so they need the
//! total duration in milliseconds. `symphonia` reads it from the first
//! track's container metadata without decoding any audio.

use std::io::Cursor;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;
use symphonia::default::get_probe;

/// Probe the duration of `bytes` in milliseconds.
///
/// `extension_hint` (e.g. `"wav"`, `"mp3"`) biases format detection. The
/// duration comes from the first track's timebase and frame count; no
/// samples are decoded.
///
/// # Errors
///
/// Returns a validation error when the container can't be probed or the
/// first track lacks the timebase/duration metadata needed to compute a
/// duration.
pub(super) fn probe_duration_ms(bytes: &Bytes, extension_hint: &str) -> Result<u64> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.clone())), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(extension_hint);

    let reader = get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| Error::new(ErrorKind::Validation, format!("audio probe failed: {e}")))?;

    let track = reader
        .tracks()
        .first()
        .ok_or_else(|| Error::new(ErrorKind::Validation, "audio probe returned no tracks"))?;

    let time_base = track
        .time_base
        .ok_or_else(|| Error::new(ErrorKind::Validation, "audio track is missing a timebase"))?;
    let duration = track
        .duration
        .ok_or_else(|| Error::new(ErrorKind::Validation, "audio track is missing a duration"))?;

    // `Track::duration` is timebase ticks (u64); `calc_time` takes a
    // signed `Timestamp` in the same unit, anchored at zero.
    let ticks = i64::try_from(duration.get())
        .map_err(|_| Error::new(ErrorKind::Validation, "audio duration overflowed i64 ticks"))?;
    let time = time_base.calc_time(Timestamp::new(ticks)).ok_or_else(|| {
        Error::new(
            ErrorKind::Validation,
            "audio duration overflowed on conversion",
        )
    })?;

    let ms = time.as_nanos() / 1_000_000;
    u64::try_from(ms).map_err(|_| {
        Error::new(
            ErrorKind::Validation,
            "audio duration is negative or overflowed",
        )
    })
}
