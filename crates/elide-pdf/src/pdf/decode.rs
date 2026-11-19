//! Decoding a stream's filters under a bound on what it decodes to.
//!
//! hayro-syntax decodes a stream to whatever size it inflates to, so a
//! compressed stream a few kilobytes long could take gigabytes. The
//! non-image filters are decoded here instead, from the stream's decrypted
//! bytes, and stop at a limit (7.4).

use std::io::Read as _;

use elide_core::{Error, Result};
use flate2::read::ZlibDecoder;
use hayro_syntax::object::{Array, Dict, Name, Object, Stream};

/// One filter of a stream, with what its parameters say decoding takes.
#[derive(Debug, Clone)]
pub(crate) struct Filter {
    pub(crate) name: Vec<u8>,
    predictor: Option<Predictor>,
    /// Whether LZW codes widen one code early (`/EarlyChange`, 1 by default).
    early_change: bool,
}

/// The filters of `stream` (`/Filter`), each with its parameters
/// (`/DecodeParms`); empty when it has none.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `/Filter` is neither a name nor an array of names, or a predictor cannot
/// be undone.
pub(crate) fn filters(stream: &Stream<'_>) -> Result<Vec<Filter>> {
    let dict = stream.dict();
    let read = |name: &[u8], params: Option<Dict<'_>>| -> Result<Filter> {
        let number = |key: &[u8]| params.as_ref().and_then(|p| p.get::<i32>(key));
        Ok(Filter {
            name: name.to_vec(),
            predictor: Predictor::read(number)?,
            early_change: number(b"EarlyChange") != Some(0),
        })
    };
    if let Some(name) = dict.get::<Name<'_>>(b"Filter") {
        return Ok(vec![read(&name, dict.get::<Dict<'_>>(b"DecodeParms"))?]);
    }
    let Some(names) = dict.get::<Array<'_>>(b"Filter") else {
        return match dict.get::<Object<'_>>(b"Filter") {
            None | Some(Object::Null(_)) => Ok(Vec::new()),
            Some(_) => Err(not_a_filter()),
        };
    };
    let params: Vec<Option<Dict<'_>>> = dict
        .get::<Array<'_>>(b"DecodeParms")
        .map(|params| params.iter::<Object<'_>>().map(Object::into_dict).collect())
        .unwrap_or_default();
    names
        .iter::<Object<'_>>()
        .enumerate()
        .map(|(i, name)| {
            let name = name.into_name().ok_or_else(not_a_filter)?;
            read(&name, params.get(i).cloned().flatten())
        })
        .collect()
}

/// A `/Filter` that is not a filter name.
fn not_a_filter() -> Error {
    Error::malformed("a stream's /Filter is not a filter name")
}

/// `stream`'s content with its filters decoded, at most `limit` bytes.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it
/// decodes past `limit`;
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// a filter is unknown, an image's, or its data does not decode.
pub(crate) fn decode(stream: &Stream<'_>, limit: usize) -> Result<Vec<u8>> {
    apply(stream.raw_data().into_owned(), &filters(stream)?, limit)
}

/// `data` with `filters` decoded in turn, at most `limit` bytes.
fn apply(mut data: Vec<u8>, filters: &[Filter], limit: usize) -> Result<Vec<u8>> {
    for filter in filters {
        data = match filter.name.as_slice() {
            b"FlateDecode" | b"Fl" => predict(inflate(&data, limit)?, filter, limit)?,
            b"LZWDecode" | b"LZW" => {
                predict(lzw(&data, filter.early_change, limit)?, filter, limit)?
            }
            b"ASCIIHexDecode" | b"AHx" => hex(&data),
            b"ASCII85Decode" | b"A85" => ascii85(&data)?,
            b"RunLengthDecode" | b"RL" => run_length(&data, limit)?,
            // Decrypted already, as the stream's bytes are read.
            b"Crypt" => data,
            name => {
                return Err(Error::malformed(format!(
                    "a stream's filter {} cannot be decoded as content",
                    String::from_utf8_lossy(name)
                )));
            }
        };
        within(data.len(), limit)?;
    }
    within(data.len(), limit)?;
    Ok(data)
}

/// Refuse a decode of `len` bytes past `limit`.
fn within(len: usize, limit: usize) -> Result<()> {
    if len > limit {
        return Err(Error::resource_limit(format!(
            "a stream decodes past its {limit}-byte limit"
        )));
    }
    Ok(())
}

/// `data` inflated (zlib), at most `limit` bytes.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| Error::malformed(format!("a Flate stream does not decode: {e}")))?;
    within(out.len(), limit)?;
    Ok(out)
}

/// `data` LZW-decoded (7.4.4), with codes widening one code early when
/// `early`, at most `limit` bytes.
fn lzw(data: &[u8], early: bool, limit: usize) -> Result<Vec<u8>> {
    const CLEAR: usize = 256;
    const END: usize = 257;
    let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    table.extend([Vec::new(), Vec::new()]);
    let mut out = Vec::new();
    let mut width = 9;
    let mut previous: Option<usize> = None;
    let (mut bits, mut held) = (0u32, 0u32);
    for &byte in data {
        bits = (bits << 8) | u32::from(byte);
        held += 8;
        while held >= width {
            let code = ((bits >> (held - width)) & ((1 << width) - 1)) as usize;
            held -= width;
            match code {
                CLEAR => {
                    table.truncate(END + 1);
                    width = 9;
                    previous = None;
                    continue;
                }
                END => return Ok(out),
                _ => {}
            }
            let entry = match (table.get(code), previous) {
                (Some(entry), _) => entry.clone(),
                (None, Some(prev)) if code == table.len() => {
                    let mut entry = table[prev].clone();
                    entry.push(table[prev][0]);
                    entry
                }
                _ => return Err(Error::malformed("an LZW stream does not decode")),
            };
            out.extend_from_slice(&entry);
            within(out.len(), limit)?;
            if let Some(prev) = previous
                && table.len() < 4096
            {
                let mut next = table[prev].clone();
                next.push(entry[0]);
                table.push(next);
            }
            previous = Some(code);
            let next = table.len() + usize::from(early);
            width = match next {
                ..512 => 9,
                512..1024 => 10,
                1024..2048 => 11,
                _ => 12,
            };
        }
    }
    Ok(out)
}

/// `data` hex-decoded (7.4.2): whitespace skipped, up to `>`, a last odd
/// digit followed by 0.
fn hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high = None;
    for &byte in data {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            b'>' => break,
            _ => continue,
        };
        match high.take() {
            Some(high) => out.push(high << 4 | digit),
            None => high = Some(digit),
        }
    }
    if let Some(high) = high {
        out.push(high << 4);
    }
    out
}

/// `data` base-85-decoded (7.4.3): whitespace skipped, `z` for four zero
/// bytes, up to `~>`.
fn ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let malformed = || Error::malformed("an ASCII85 stream does not decode");
    let mut out = Vec::with_capacity(data.len() / 5 * 4);
    let mut group = [0u8; 5];
    let mut len = 0;
    let push = |group: &[u8; 5], len: usize, out: &mut Vec<u8>| -> Result<()> {
        let mut value: u64 = 0;
        for &digit in group {
            value = value * 85 + u64::from(digit);
        }
        let value = u32::try_from(value).map_err(|_| malformed())?;
        out.extend_from_slice(&value.to_be_bytes()[..len - 1]);
        Ok(())
    };
    for &byte in data {
        match byte {
            b'~' => break,
            b'z' if len == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[len] = byte - b'!';
                len += 1;
                if len == 5 {
                    push(&group, 5, &mut out)?;
                    len = 0;
                }
            }
            byte if byte.is_ascii_whitespace() || byte == 0 => {}
            _ => return Err(malformed()),
        }
    }
    match len {
        0 => {}
        1 => return Err(malformed()),
        len => {
            group[len..].fill(84);
            push(&group, len, &mut out)?;
        }
    }
    Ok(out)
}

/// `data` run-length-decoded (7.4.5), at most `limit` bytes.
fn run_length(data: &[u8], limit: usize) -> Result<Vec<u8>> {
    let malformed = || Error::malformed("a RunLength stream does not decode");
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(&length) = data.get(at) {
        at += 1;
        match length {
            128 => break,
            0..128 => {
                let count = usize::from(length) + 1;
                let run = data.get(at..at + count).ok_or_else(malformed)?;
                out.extend_from_slice(run);
                at += count;
            }
            _ => {
                let byte = *data.get(at).ok_or_else(malformed)?;
                out.resize(out.len() + 257 - usize::from(length), byte);
                at += 1;
            }
        }
        within(out.len(), limit)?;
    }
    Ok(out)
}

/// A filter's predictor parameters (7.4.4.4): the predictor, and the
/// layout of the samples it predicts.
#[derive(Debug, Clone, Copy)]
struct Predictor {
    kind: usize,
    colors: usize,
    bits: usize,
    columns: usize,
}

impl Predictor {
    /// The predictor parameters `number` reads name, `None` when they
    /// name none.
    fn read(number: impl Fn(&[u8]) -> Option<i32>) -> Result<Option<Self>> {
        let number = |key: &[u8], default: usize| {
            number(key).map_or(Some(default), |v| usize::try_from(v).ok())
        };
        let kind = number(b"Predictor", 1);
        let colors = number(b"Colors", 1).filter(|&c| (1..=32).contains(&c));
        let bits = number(b"BitsPerComponent", 8).filter(|b| matches!(b, 1 | 2 | 4 | 8 | 16));
        let columns = number(b"Columns", 1).filter(|&c| c >= 1);
        match (kind, colors, bits, columns) {
            (Some(1), ..) => Ok(None),
            (Some(kind), Some(colors), Some(bits), Some(columns)) => Ok(Some(Self {
                kind,
                colors,
                bits,
                columns,
            })),
            _ => Err(unusable()),
        }
    }

    /// `data` with the prediction undone, at most `limit` bytes.
    fn undo(self, data: Vec<u8>, limit: usize) -> Result<Vec<u8>> {
        let row = (self.colors * self.bits)
            .checked_mul(self.columns)
            .map(|bits| bits.div_ceil(8))
            .filter(|&row| row <= limit)
            .ok_or_else(unusable)?;
        // The bytes from one sample to the next of the same component.
        let step = (self.colors * self.bits).div_ceil(8);
        match self.kind {
            2 if self.bits == 8 => {
                let mut out = data;
                for line in out.chunks_mut(row) {
                    for i in step..line.len() {
                        line[i] = line[i].wrapping_add(line[i - step]);
                    }
                }
                Ok(out)
            }
            10..=15 => {
                let mut out = Vec::with_capacity(data.len());
                let mut above = vec![0u8; row];
                for line in data.chunks(row + 1) {
                    let (&kind, encoded) = line.split_first().ok_or_else(unusable)?;
                    let mut current = encoded.to_vec();
                    current.resize(row, 0);
                    for i in 0..row {
                        let left = if i >= step { current[i - step] } else { 0 };
                        let up = above[i];
                        let corner = if i >= step { above[i - step] } else { 0 };
                        current[i] = current[i].wrapping_add(match kind {
                            0 => 0,
                            1 => left,
                            2 => up,
                            3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                            4 => paeth(left, up, corner),
                            _ => return Err(unusable()),
                        });
                    }
                    out.extend_from_slice(&current[..encoded.len()]);
                    above = current;
                }
                within(out.len(), limit)?;
                Ok(out)
            }
            _ => Err(unusable()),
        }
    }
}

/// A predictor that cannot be undone.
fn unusable() -> Error {
    Error::malformed("a stream's predictor parameters are not usable")
}

/// `data` with `filter`'s predictor undone.
fn predict(data: Vec<u8>, filter: &Filter, limit: usize) -> Result<Vec<u8>> {
    match filter.predictor {
        Some(predictor) => predictor.undo(data, limit),
        None => Ok(data),
    }
}

/// The PNG Paeth predictor of a byte from its left, upper and upper-left
/// neighbours.
fn paeth(left: u8, up: u8, corner: u8) -> u8 {
    let estimate = i16::from(left) + i16::from(up) - i16::from(corner);
    let (to_left, to_up, to_corner) = (
        (estimate - i16::from(left)).abs(),
        (estimate - i16::from(up)).abs(),
        (estimate - i16::from(corner)).abs(),
    );
    if to_left <= to_up && to_left <= to_corner {
        left
    } else if to_up <= to_corner {
        up
    } else {
        corner
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;

    fn deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// A stream inflating past its limit is refused, not decoded whole.
    #[test]
    fn inflating_past_the_limit_is_refused() {
        let bomb = deflate(&vec![0; 1 << 20]);
        assert!(inflate(&bomb, 1 << 20).is_ok());
        let err = inflate(&bomb, 1 << 10).unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::ResourceLimit);
    }

    #[test]
    fn ascii_filters_decode() {
        assert_eq!(hex(b"48 65 6c6C6f>junk"), b"Hello");
        assert_eq!(hex(b"7"), [0x70]);
        assert_eq!(ascii85(b"87cURD]i,\"Ebo80~>").unwrap(), b"Hello World!");
        assert_eq!(ascii85(b"z~>").unwrap(), [0; 4]);
    }

    #[test]
    fn run_length_and_lzw_decode() {
        assert_eq!(
            run_length(&[2, b'a', b'b', b'c', 254, b'x', 128], 64).unwrap(),
            b"abcxxx"
        );
        assert!(run_length(&[129, b'x'], 4).is_err());
        // "-----A---B" from the example in 7.4.4.2, early change on.
        let encoded = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&encoded, true, 64).unwrap(), b"-----A---B");
    }

    /// PNG rows are each undone by their own filter type; TIFF rows add
    /// each sample to the one before.
    #[test]
    fn predictors_are_undone() {
        let png = Predictor {
            kind: 12,
            colors: 1,
            bits: 8,
            columns: 3,
        };
        let encoded = vec![0, 5, 6, 7, 2, 1, 1, 1, 1, 1, 1, 1];
        assert_eq!(png.undo(encoded, 64).unwrap(), [5, 6, 7, 6, 7, 8, 1, 2, 3]);
        let tiff = Predictor { kind: 2, ..png };
        assert_eq!(
            tiff.undo(vec![1, 1, 1, 2, 2, 2], 64).unwrap(),
            [1, 2, 3, 2, 4, 6]
        );
        assert!(Predictor { kind: 99, ..png }.undo(vec![0], 64).is_err());
    }
}
