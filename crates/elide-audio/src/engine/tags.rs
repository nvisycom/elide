//! Container metadata strip: drop the tag blocks an audio file carries beside
//! its samples.
//!
//! An MP3 carries ID3v2 tags at its start (and rarely, with a footer, at its
//! end), an ID3v1 tag, an APEv2 tag, and a Lyrics3 block at its end: titles,
//! artists, comments, cover art, encoder settings. A WAV carries `LIST/INFO`,
//! `id3 `, broadcast (`bext`), iXML, and XMP chunks beside its format and
//! sample data. None of these is redacted field by field, so they are removed
//! wholesale. The WAV strip is an allowlist of the chunks that play the audio;
//! the MP3 strip peels every known tag block off the frame stream's two ends.

#[cfg(feature = "mp3")]
use std::str;

use elide_core::{Error, ErrorKind, Result};

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
                    None => match id3v2_footer_tag(tail)? {
                        Some(len) => len,
                        None => break,
                    },
                },
            },
        };
        end -= len;
        tags += 1;
    }
    Ok(Untagged {
        bytes: bytes[start..end].to_vec(),
        tags,
    })
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
    let header = if le(20) & 0x8000_0000 != 0 { 32 } else { 0 };
    let len = le(12) as usize + header;
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
    let mut at = 12;
    while at < bytes.len() {
        let header = bytes
            .get(at..at + 8)
            .ok_or_else(|| malformed("truncated WAV chunk header"))?;
        let size = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes")) as usize;
        // A chunk's data is padded to an even length.
        let end = at + 8 + size + size % 2;
        let end = within(end, bytes.len(), "WAV chunk")?;
        if WAV_PLAYBACK_CHUNKS.contains(&header[..4].try_into().expect("4 bytes")) {
            chunks.extend_from_slice(&bytes[at..end]);
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
    Ok(Untagged { bytes: out, tags })
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
        use super::*;

        const FRAMES: &[u8] = b"\xFF\xFBframe-data";

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
            assert_eq!(untagged.tags, 3);
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
