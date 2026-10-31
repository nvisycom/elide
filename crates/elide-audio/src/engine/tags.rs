//! Container metadata strip: drop the tag blocks an audio file carries beside
//! its samples.
//!
//! An MP3 carries ID3v2 tags at its start (and rarely, with a footer, at its
//! end), an ID3v1 tag, an APEv2 tag, and a Lyrics3 block at its end: titles,
//! artists, comments, cover art, encoder settings. A WAV carries `LIST/INFO`,
//! `id3 `, broadcast (`bext`), iXML, and XMP chunks beside its format and
//! sample data. None of these is redacted field by field, so they are removed
//! wholesale. The WAV strip is an allowlist of the chunks that play the audio;
//! the MP3 strip peels every known tag block off the frame stream's two ends,
//! then blanks the encoder version a LAME tag records in the first frame.

#[cfg(feature = "mp3")]
use std::str;

use elide_core::{Error, ErrorKind, Result};
#[cfg(feature = "mp3")]
use symphonia::core::checksum::Crc16AnsiLe;
#[cfg(feature = "mp3")]
use symphonia::core::io::Monitor;

use crate::buffer::Untagged;

/// Remove every tag block from the MP3 `bytes`, leaving the frame stream.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if a tag block's declared size overruns the
/// file.
#[cfg(feature = "mp3")]
pub(crate) fn strip_mp3(bytes: &[u8]) -> Result<Untagged> {
    let mut tags = 0;
    let mut start = 0;
    while let Some(len) = id3v2_header_tag(&bytes[start..])? {
        start += len;
        tags += 1;
    }
    let mut end = bytes.len();
    loop {
        let tail = &bytes[start..end];
        let len = match id3v1_tag(tail) {
            Some(len) => len,
            None => match apev2_tag(tail)? {
                Some(len) => len,
                None => match lyrics3v2_tag(tail)? {
                    Some(len) => len,
                    None => match lyrics3v1_tag(tail) {
                        Some(len) => len,
                        None => match id3v2_footer_tag(tail)? {
                            Some(len) => len,
                            None => break,
                        },
                    },
                },
            },
        };
        end -= len;
        tags += 1;
    }
    let mut frames = bytes[start..end].to_vec();
    tags += usize::from(blank_encoder_version(&mut frames));
    Ok(Untagged {
        bytes: frames,
        tags,
        padding: 0,
    })
}

/// The length of a LAME extension, the last two bytes of which are its CRC.
#[cfg(feature = "mp3")]
const LAME_EXTENSION_LEN: usize = 36;

/// Blank the encoder version a LAME tag records in an MP3's first frame,
/// returning whether there was one to blank.
///
/// The Xing/Info frame opening an MP3 is a silent frame that holds what a player
/// needs (the frame count and seek table, the encoder delay and padding for
/// gapless playback) and, in its LAME extension, the encoder's name and version
/// (`LAME3.100`, `Lavc58.91`). Decoders read the delay and padding only when
/// the name is a known encoder, so the four-byte family stays and only the
/// version that fingerprints the tool chain is blanked. The tag's CRC is then
/// recomputed, so a decoder still trusts the rest of the tag.
#[cfg(feature = "mp3")]
fn blank_encoder_version(frames: &mut [u8]) -> bool {
    let Some(at) = lame_extension(frames) else {
        return false;
    };
    let version = &mut frames[at + 4..at + 9];
    if version.iter().all(|&b| b == b' ') {
        return false;
    }
    version.fill(b' ');
    // The CRC covers the frame up to itself; one written as zero is ignored by
    // decoders, so it stays zero.
    let crc_at = at + LAME_EXTENSION_LEN - 2;
    if let Some(stored) = frames.get(crc_at..crc_at + 2)
        && stored != [0, 0]
    {
        let mut crc = Crc16AnsiLe::new(0);
        crc.process_buf_bytes(&frames[..crc_at]);
        frames[crc_at..crc_at + 2].copy_from_slice(&crc.crc().to_be_bytes());
    }
    true
}

/// Where the LAME extension of the Xing/Info tag in the frame opening `frames`
/// starts, or `None` when the stream opens with no such tag.
///
/// The tag follows the frame header and the side information, whose length
/// depends on the MPEG version and channel mode; its flags say which optional
/// fields (frame count, byte count, seek table, quality) precede the extension.
#[cfg(feature = "mp3")]
fn lame_extension(frames: &[u8]) -> Option<usize> {
    let header = u32::from_be_bytes(frames.get(..4)?.try_into().ok()?);
    let synced = header >> 21 == 0x7FF;
    let version = (header >> 19) & 0b11;
    let layer_three = (header >> 17) & 0b11 == 0b01;
    if !synced || version == 0b01 || !layer_three {
        return None;
    }
    let mpeg1 = version == 0b11;
    let mono = (header >> 6) & 0b11 == 0b11;
    let side_info = match (mpeg1, mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let tag = 4 + side_info;
    let id = frames.get(tag..tag + 4)?;
    if id != b"Xing" && id != b"Info" {
        return None;
    }
    let flags = u32::from_be_bytes(frames.get(tag + 4..tag + 8)?.try_into().ok()?);
    let optional = [(0x1, 4), (0x2, 4), (0x4, 100), (0x8, 4)]
        .into_iter()
        .filter(|&(flag, _)| flags & flag != 0)
        .map(|(_, len)| len)
        .sum::<usize>();
    let at = tag + 8 + optional;
    // The encoder field must be whole to edit it.
    frames.get(at..at + 9)?;
    Some(at)
}

/// The length of the ID3v2 tag opening `bytes`, or `None` when it opens with
/// none.
#[cfg(feature = "mp3")]
fn id3v2_header_tag(bytes: &[u8]) -> Result<Option<usize>> {
    if bytes.len() < 10 || !bytes.starts_with(b"ID3") {
        return Ok(None);
    }
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    let len = 10 + syncsafe(&bytes[6..10])? + footer;
    within(len, bytes.len(), "ID3v2 tag").map(Some)
}

/// The length of the ID3v2 tag closing `bytes` with a `3DI` footer, or `None`.
#[cfg(feature = "mp3")]
fn id3v2_footer_tag(bytes: &[u8]) -> Result<Option<usize>> {
    let Some(footer) = bytes.len().checked_sub(10).map(|at| &bytes[at..]) else {
        return Ok(None);
    };
    if !footer.starts_with(b"3DI") {
        return Ok(None);
    }
    let len = 20 + syncsafe(&footer[6..10])?;
    within(len, bytes.len(), "ID3v2 footer tag").map(Some)
}

/// The length of the ID3v1 tag (with its enhanced `TAG+` block, when present)
/// closing `bytes`, or `None`.
#[cfg(feature = "mp3")]
fn id3v1_tag(bytes: &[u8]) -> Option<usize> {
    let at = bytes.len().checked_sub(128)?;
    if !bytes[at..].starts_with(b"TAG") {
        return None;
    }
    let enhanced = at
        .checked_sub(227)
        .is_some_and(|plus| bytes[plus..].starts_with(b"TAG+"));
    Some(if enhanced { 128 + 227 } else { 128 })
}

/// The length of the APEv2 tag (footer, items, and optional header) closing
/// `bytes`, or `None`.
#[cfg(feature = "mp3")]
fn apev2_tag(bytes: &[u8]) -> Result<Option<usize>> {
    let Some(footer) = bytes.len().checked_sub(32).map(|at| &bytes[at..]) else {
        return Ok(None);
    };
    if !footer.starts_with(b"APETAGEX") {
        return Ok(None);
    }
    let le = |at: usize| u32::from_le_bytes(footer[at..at + 4].try_into().expect("4 bytes"));
    // The size covers the items and the footer, so it is never under the
    // footer's own 32 bytes; a smaller one would peel nothing off and loop.
    let size = le(12) as usize;
    if size < 32 {
        return Err(malformed("APEv2 tag size is smaller than its footer"));
    }
    let header = if le(20) & 0x8000_0000 != 0 { 32 } else { 0 };
    let len = size + header;
    within(len, bytes.len(), "APEv2 tag").map(Some)
}

/// The length of the Lyrics3v2 block closing `bytes`, or `None`.
#[cfg(feature = "mp3")]
fn lyrics3v2_tag(bytes: &[u8]) -> Result<Option<usize>> {
    const END: &[u8] = b"LYRICS200";
    if !bytes.ends_with(END) || bytes.len() < END.len() + 6 {
        return Ok(None);
    }
    let digits = &bytes[bytes.len() - END.len() - 6..bytes.len() - END.len()];
    let size = str::from_utf8(digits)
        .ok()
        .and_then(|d| d.parse::<usize>().ok())
        .ok_or_else(|| malformed("Lyrics3 block size is not a number"))?;
    let len = within(size + 6 + END.len(), bytes.len(), "Lyrics3 block")?;
    if !bytes[bytes.len() - len..].starts_with(b"LYRICSBEGIN") {
        return Err(malformed("Lyrics3 block does not open with LYRICSBEGIN"));
    }
    Ok(Some(len))
}

/// The length of the Lyrics3 (v1) block closing `bytes`, or `None`: free-text
/// lyrics between `LYRICSBEGIN` and `LYRICSEND`, at most 5100 bytes of them,
/// with no size field, so the start is found by searching back for its marker.
#[cfg(feature = "mp3")]
fn lyrics3v1_tag(bytes: &[u8]) -> Option<usize> {
    const BEGIN: &[u8] = b"LYRICSBEGIN";
    const END: &[u8] = b"LYRICSEND";
    const MAX_LYRICS: usize = 5100;
    if !bytes.ends_with(END) {
        return None;
    }
    let window = (BEGIN.len() + MAX_LYRICS + END.len()).min(bytes.len());
    let tail = &bytes[bytes.len() - window..];
    let begin = tail.windows(BEGIN.len()).rposition(|w| w == BEGIN)?;
    Some(window - begin)
}

/// Decode a 4-byte ID3v2 syncsafe integer (7 significant bits per byte).
#[cfg(feature = "mp3")]
fn syncsafe(bytes: &[u8]) -> Result<usize> {
    bytes.iter().try_fold(0usize, |acc, &b| {
        if b & 0x80 != 0 {
            return Err(malformed("ID3v2 size is not syncsafe"));
        }
        Ok((acc << 7) | usize::from(b))
    })
}

/// The chunks of a WAV kept by the strip: the format description, the sample
/// count of a compressed stream, and the samples themselves.
#[cfg(feature = "wav")]
const WAV_PLAYBACK_CHUNKS: [&[u8; 4]; 3] = [b"fmt ", b"fact", b"data"];

/// The padding chunks a WAV writer reserves space with. They hold no metadata
/// of their own, but can keep stale bytes from an earlier edit, so they are
/// stripped and counted apart from the tags.
#[cfg(feature = "wav")]
const WAV_PADDING_CHUNKS: [&[u8; 4]; 3] = [b"JUNK", b"PAD ", b"FLLR"];

/// Remove every chunk from the WAV `bytes` but the ones that play it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the bytes are not a RIFF `WAVE` file or a
/// chunk overruns it.
#[cfg(feature = "wav")]
pub(crate) fn strip_wav(bytes: &[u8]) -> Result<Untagged> {
    if bytes.len() < 12 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WAVE" {
        return Err(malformed("not a RIFF WAVE file"));
    }
    let mut chunks = Vec::with_capacity(bytes.len());
    let mut tags = 0;
    let mut padding = 0;
    let mut at = 12;
    while at < bytes.len() {
        let header = bytes
            .get(at..at + 8)
            .ok_or_else(|| malformed("truncated WAV chunk header"))?;
        let size = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes")) as usize;
        // A chunk's data is padded to an even length.
        let end = at + 8 + size + size % 2;
        let end = within(end, bytes.len(), "WAV chunk")?;
        let id: &[u8; 4] = header[..4].try_into().expect("4 bytes");
        if WAV_PLAYBACK_CHUNKS.contains(&id) {
            chunks.extend_from_slice(&bytes[at..end]);
        } else if WAV_PADDING_CHUNKS.contains(&id) {
            padding += 1;
        } else {
            tags += 1;
        }
        at = end;
    }
    let riff_size = u32::try_from(chunks.len() + 4)
        .map_err(|_| malformed("WAV larger than a RIFF file can describe"))?;
    let mut out = Vec::with_capacity(chunks.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(&chunks);
    Ok(Untagged {
        bytes: out,
        tags,
        padding,
    })
}

/// `len` when it fits in `available` bytes, else a malformed error naming
/// `what` overran.
fn within(len: usize, available: usize, what: &str) -> Result<usize> {
    if len > available {
        return Err(malformed(&format!("{what} overruns the file")));
    }
    Ok(len)
}

/// A malformed-container error with `message`.
fn malformed(message: &str) -> Error {
    Error::new(
        ErrorKind::MalformedInput,
        format!("audio metadata strip: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "mp3")]
    mod mp3 {
        use std::io::Cursor;

        use symphonia::core::formats::probe::Hint;
        use symphonia::core::formats::{FormatOptions, TrackType};
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::default::get_probe;

        use super::*;
        use crate::fixtures;

        const FRAMES: &[u8] = b"\xFF\xFBframe-data";

        /// The encoder delay and padding the test LAME tag records.
        const DELAY: u32 = 576;
        const PADDING: u32 = 100;

        /// `audio` (a raw LAME stream of 64 kbit/s, 16 kHz mono MPEG-2 frames)
        /// opened by an Info frame whose LAME extension names `LAME3.100` and
        /// records [`DELAY`] and [`PADDING`], with a valid tag CRC.
        fn with_lame_tag(audio: &[u8]) -> Vec<u8> {
            // The audio's own header, padding bit cleared: 72 * 64000 / 16000.
            let mut frame = audio[..4].to_vec();
            frame[2] &= !0b10;
            frame.resize(4 + 9, 0);
            frame.extend_from_slice(b"Info");
            frame.extend(0u32.to_be_bytes());
            let extension = frame.len();
            frame.extend_from_slice(b"LAME3.100");
            // Revision, lowpass, peak, two gains, flags, and bitrate.
            frame.extend([0; 12]);
            let trim = ((DELAY - 529) << 12) | (PADDING + 529);
            frame.extend(&trim.to_be_bytes()[1..]);
            frame.extend([0; 10]);
            let crc_at = extension + LAME_EXTENSION_LEN - 2;
            let mut crc = Crc16AnsiLe::new(0);
            crc.process_buf_bytes(&frame[..crc_at]);
            frame.extend(crc.crc().to_be_bytes());
            frame.resize(288, 0);
            frame.extend_from_slice(audio);
            frame
        }

        /// The encoder delay and padding a decoder reads from `bytes`.
        fn gapless(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
            let source = Cursor::new(bytes.to_vec());
            let mss = MediaSourceStream::new(Box::new(source), Default::default());
            let mut hint = Hint::new();
            hint.with_extension("mp3");
            let reader = get_probe()
                .probe(
                    &hint,
                    mss,
                    FormatOptions::default(),
                    MetadataOptions::default(),
                )
                .expect("probe");
            let track = reader.default_track(TrackType::Audio).expect("track");
            (track.delay, track.padding)
        }

        #[test]
        fn the_encoder_version_is_blanked_and_gapless_playback_kept() {
            let bytes = with_lame_tag(&fixtures::mp3_tone(1));
            assert_eq!(gapless(&bytes), (Some(DELAY), Some(PADDING)));

            let untagged = strip_mp3(&bytes).unwrap();
            assert_eq!(untagged.tags, 1);
            let at = lame_extension(&untagged.bytes).expect("extension");
            assert_eq!(&untagged.bytes[at..at + 9], b"LAME     ");
            // The recomputed CRC still verifies, so the decoder keeps the trim.
            assert_eq!(gapless(&untagged.bytes), (Some(DELAY), Some(PADDING)));
            // Only the version changed.
            let changed = bytes
                .iter()
                .zip(&untagged.bytes)
                .filter(|(a, b)| a != b)
                .count();
            assert!(changed <= 5 + 2, "{changed} bytes changed");

            // A blanked tag has nothing left to strip.
            assert_eq!(strip_mp3(&untagged.bytes).unwrap().tags, 0);
        }

        #[test]
        fn a_stream_without_a_lame_tag_is_unchanged() {
            let tone = fixtures::mp3_tone(1);
            let untagged = strip_mp3(&tone).unwrap();
            assert_eq!(untagged.tags, 0);
            assert_eq!(untagged.bytes, tone.to_vec());
        }

        /// An ID3v2.3 tag around `body`.
        fn id3v2(body: &[u8]) -> Vec<u8> {
            let mut out = b"ID3\x03\x00\x00".to_vec();
            let n = body.len();
            out.extend([
                (n >> 21) as u8 & 0x7F,
                (n >> 14) as u8 & 0x7F,
                (n >> 7) as u8 & 0x7F,
                n as u8 & 0x7F,
            ]);
            out.extend_from_slice(body);
            out
        }

        /// An ID3v1 tag naming `artist`.
        fn id3v1(artist: &str) -> Vec<u8> {
            let mut out = b"TAG".to_vec();
            out.resize(33, 0);
            out.extend_from_slice(artist.as_bytes());
            out.resize(128, 0);
            out
        }

        /// An APEv2 tag with a header, around `items`.
        fn apev2(items: &[u8]) -> Vec<u8> {
            let block = |flags: u32| {
                let mut b = b"APETAGEX".to_vec();
                b.extend(2000u32.to_le_bytes());
                b.extend(((items.len() + 32) as u32).to_le_bytes());
                b.extend(1u32.to_le_bytes());
                b.extend(flags.to_le_bytes());
                b.extend([0; 8]);
                b
            };
            let mut out = block(0xA000_0000);
            out.extend_from_slice(items);
            out.extend(block(0x8000_0000));
            out
        }

        #[test]
        fn strips_every_tag_block_from_both_ends() {
            let mut bytes = id3v2(b"TPE1 Jane Roe");
            bytes.extend(id3v2(b"APIC cover"));
            bytes.extend_from_slice(FRAMES);
            bytes.extend(apev2(b"Artist=Jane"));
            bytes.extend(id3v1("Jane"));
            let untagged = strip_mp3(&bytes).unwrap();
            assert_eq!(untagged.bytes, FRAMES);
            assert_eq!(untagged.tags, 4);
        }

        #[test]
        fn strips_a_lyrics3_v1_block() {
            let mut bytes = FRAMES.to_vec();
            bytes.extend_from_slice(b"LYRICSBEGINJane Roe, verse oneLYRICSEND");
            bytes.extend(id3v1("Jane"));
            let untagged = strip_mp3(&bytes).unwrap();
            assert_eq!(untagged.bytes, FRAMES);
            assert_eq!(untagged.tags, 2);
        }

        /// An APEv2 footer claiming a size under its own 32 bytes would peel
        /// nothing off; it is refused rather than looped on.
        #[test]
        fn rejects_an_apev2_tag_smaller_than_its_footer() {
            let mut bytes = FRAMES.to_vec();
            let mut footer = b"APETAGEX".to_vec();
            footer.extend(2000u32.to_le_bytes());
            footer.extend(0u32.to_le_bytes());
            footer.extend(0u32.to_le_bytes());
            footer.extend(0u32.to_le_bytes());
            footer.extend([0; 8]);
            bytes.extend(footer);
            assert!(strip_mp3(&bytes).is_err());
        }

        #[test]
        fn strips_a_lyrics3_block() {
            let mut bytes = FRAMES.to_vec();
            let lyrics = b"LYRICSBEGININD00002Jane";
            bytes.extend_from_slice(lyrics);
            bytes.extend(format!("{:06}LYRICS200", lyrics.len()).as_bytes());
            assert_eq!(strip_mp3(&bytes).unwrap().bytes, FRAMES);
        }

        #[test]
        fn leaves_an_untagged_stream_unchanged() {
            let untagged = strip_mp3(FRAMES).unwrap();
            assert_eq!((untagged.bytes.as_slice(), untagged.tags), (FRAMES, 0));
        }

        #[test]
        fn rejects_a_tag_that_overruns_the_file() {
            let mut bytes = b"ID3\x03\x00\x00\x00\x00\x7F\x7F".to_vec();
            bytes.extend_from_slice(FRAMES);
            assert!(strip_mp3(&bytes).is_err());
        }
    }

    #[cfg(feature = "wav")]
    mod wav {
        use super::*;

        /// A RIFF chunk: id, little-endian size, data, pad byte when odd.
        fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut out = id.to_vec();
            out.extend((data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
            if data.len() % 2 == 1 {
                out.push(0);
            }
            out
        }

        /// A RIFF WAVE file of `chunks`.
        fn wav(chunks: &[Vec<u8>]) -> Vec<u8> {
            let body: Vec<u8> = chunks.concat();
            let mut out = b"RIFF".to_vec();
            out.extend(((body.len() + 4) as u32).to_le_bytes());
            out.extend_from_slice(b"WAVE");
            out.extend(body);
            out
        }

        #[test]
        fn keeps_only_the_playback_chunks() {
            let fmt = chunk(b"fmt ", &[1; 16]);
            let data = chunk(b"data", &[2; 7]);
            let bytes = wav(&[
                fmt.clone(),
                chunk(b"LIST", b"INFOIART\x05\x00\x00\x00Jane\x00"),
                chunk(b"bext", b"originator Jane"),
                data.clone(),
                chunk(b"id3 ", b"ID3 tag"),
            ]);
            let untagged = strip_wav(&bytes).unwrap();
            assert_eq!(untagged.bytes, wav(&[fmt, data]));
            assert_eq!((untagged.tags, untagged.padding), (3, 0));
        }

        #[test]
        fn padding_is_stripped_and_counted_apart_from_tags() {
            let fmt = chunk(b"fmt ", &[1; 16]);
            let data = chunk(b"data", &[2; 4]);
            let bytes = wav(&[
                fmt.clone(),
                chunk(b"JUNK", b"stale bytes from an earlier save"),
                data.clone(),
            ]);
            let untagged = strip_wav(&bytes).unwrap();
            assert_eq!(untagged.bytes, wav(&[fmt, data]));
            assert_eq!((untagged.tags, untagged.padding), (0, 1));
        }

        #[test]
        fn rejects_a_chunk_that_overruns_the_file() {
            let mut bytes = wav(&[chunk(b"fmt ", &[1; 16])]);
            bytes.extend(b"data\xFF\x00\x00\x00");
            assert!(strip_wav(&bytes).is_err());
            assert!(strip_wav(b"not a wav").is_err());
        }
    }
}
