//! Real documents: those `testdata/corpus/fetched.tsv` lists, which
//! `scripts/pdf/fetch-corpus.py` fetches into the directory
//! `ELIDE_PDF_CORPUS` names, and those generated into `testdata/corpus`,
//! each page compared as [`super`] compares a case's.
//!
//! A document or page the codec doesn't read from its text layer (one it
//! can't open or rebuild, or whose text it refuses or can't place, which it
//! redacts from its render instead) is passed over and counted, as is a
//! glyph placed alike but read unlike. One PDFium can't read fails, and so
//! does a fetched corpus none of whose pages is compared.

use std::path::{Path, PathBuf};
use std::{env, fs};

use super::{
    compare_rebuilt, compare_source, hidden, placed, reads_programs, rebuilt, theirs, widths_alike,
};
use crate::content::text::Text;
use crate::pdf::Pdf;

/// The variable naming the directory the fetched corpus is in.
const CORPUS: &str = "ELIDE_PDF_CORPUS";

/// The fetched corpus's manifest: a line per document, its name first, and
/// `#` lines of comment.
const FETCHED: &str = include_str!("../../../../testdata/corpus/fetched.tsv");

/// The most disagreements reported of one page.
const MAX_REPORTED: usize = 5;

/// What comparing one document came to.
#[derive(Default)]
struct Outcome {
    /// Pages compared.
    compared: usize,
    /// Pages passed over, each with why.
    passed_over: Vec<(u32, String)>,
    /// Disagreements, each on its page.
    errors: Vec<(u32, String)>,
    /// Glyphs placed alike but read unlike, each on its page.
    readings: Vec<(u32, String)>,
}

/// How one document was read.
enum Reading {
    /// The codec doesn't read it from its text layer, and why.
    Unread(String),
    /// What comparing its pages came to.
    Compared(Outcome),
}

/// Compare every page of the document `bytes`, unless the codec doesn't
/// read it from its text layer.
///
/// # Errors
///
/// Why it can't be compared: PDFium can't read it, or reads fewer pages.
fn document(bytes: &[u8]) -> Result<Reading, String> {
    let unread = |why: String| Ok(Reading::Unread(why));
    let source = match Pdf::open(bytes) {
        Ok(source) => source,
        Err(e) => return unread(format!("it can't be opened: {e}")),
    };
    let rebuilt = match rebuilt(&source) {
        Ok(rebuilt) => rebuilt,
        Err(why) => return unread(why),
    };
    let blocks = match rebuilt.read(Text) {
        Ok(blocks) => blocks,
        Err(e) => return unread(format!("the reader refuses it: {e}")),
    };
    let rebuilt_pages = theirs(&rebuilt)?;
    let source_pages = if widths_alike(source.doc()) && reads_programs(source.doc()) {
        Some(theirs(&source)?)
    } else {
        None
    };
    let mut outcome = Outcome::default();
    for block in &blocks {
        let page = block.page;
        if block.unplaced {
            outcome
                .passed_over
                .push((page, "its glyphs can't be placed".into()));
            continue;
        }
        // A page the reader places nothing on is compared too: text PDFium
        // reads there is text the reader lost.
        let ours = placed(block)?;
        let index = page as usize - 1;
        let theirs = rebuilt_pages
            .get(index)
            .ok_or_else(|| format!("PDFium reads no page {page}"))?;
        let mut errors = Vec::new();
        let mut readings = Vec::new();
        compare_rebuilt(&ours, theirs, &mut errors, &mut readings, hidden(block));
        outcome
            .readings
            .extend(readings.into_iter().take(MAX_REPORTED).map(|r| (page, r)));
        let mut found: Vec<String> = errors.drain(..).map(|e| format!("(rebuilt) {e}")).collect();
        if let Some(theirs) = source_pages.as_ref().and_then(|pages| pages.get(index)) {
            compare_source(&ours, theirs, hidden(block), &mut errors);
            found.extend(errors.into_iter().map(|e| format!("(source) {e}")));
        }
        outcome.compared += 1;
        let more = found.len().saturating_sub(MAX_REPORTED);
        outcome
            .errors
            .extend(found.into_iter().take(MAX_REPORTED).map(|e| (page, e)));
        if more > 0 {
            outcome.errors.push((page, format!("and {more} more")));
        }
    }
    Ok(Reading::Compared(outcome))
}

/// The documents [`FETCHED`] lists, in `dir`.
///
/// # Panics
///
/// If one isn't there: the fetch script fetches them.
fn fetched(dir: &Path) -> Vec<PathBuf> {
    let listed: Vec<PathBuf> = FETCHED
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('\t').next())
        .map(|name| dir.join(name))
        .collect();
    let missing: Vec<String> = listed
        .iter()
        .filter(|path| !path.is_file())
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "run scripts/pdf/fetch-corpus.py: {CORPUS} lacks {} of the {} documents, {} among them",
        missing.len(),
        listed.len(),
        missing[..missing.len().min(3)].join(", ")
    );
    listed
}

/// The PDF files under `dir`, by path.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                dirs.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The reader places every glyph of every page of the corpus the codec
/// reads from its text layer where PDFium does.
#[test]
#[ignore = "requires the native PDFium library and the corpus scripts/pdf/fetch-corpus.py fetches"]
fn documents_are_placed_as_pdfium_places_them() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // A relative path is the workspace's, where the fetch script writes.
    let Some(dir) = env::var_os(CORPUS) else {
        panic!("{CORPUS} names no directory: run scripts/pdf/fetch-corpus.py");
    };
    let fetched = fetched(&crate_dir.join("../..").join(dir));
    let generated = files(&crate_dir.join("testdata/corpus"));
    let fetched_count = fetched.len();
    let (mut documents, mut compared, mut passed_over, mut read_unlike) = (0, 0, 0, 0);
    let mut fetched_compared = 0;
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for (index, path) in [fetched, generated].concat().into_iter().enumerate() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(&path).unwrap();
        documents += 1;
        let outcome = match document(&bytes) {
            Ok(Reading::Compared(outcome)) => outcome,
            Ok(Reading::Unread(why)) => {
                report.push(format!("{name}: passed over: {why}"));
                continue;
            }
            Err(why) => {
                failures.push(format!("{name}: {why}"));
                continue;
            }
        };
        compared += outcome.compared;
        if index < fetched_count {
            fetched_compared += outcome.compared;
        }
        read_unlike += outcome.readings.len();
        for (page, reading) in &outcome.readings {
            report.push(format!("{name} p{page}: read unlike: {reading}"));
        }
        passed_over += outcome.passed_over.len();
        for (page, why) in &outcome.passed_over {
            report.push(format!("{name} p{page}: passed over: {why}"));
        }
        failures.extend(
            outcome
                .errors
                .into_iter()
                .map(|(page, error)| format!("{name} p{page}: {error}")),
        );
    }
    println!(
        "{documents} documents, {compared} pages compared, {passed_over} passed over, \
         {read_unlike} glyphs read unlike\n{}",
        report.join("\n")
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(
        fetched_compared > 0,
        "no page of the fetched corpus was compared"
    );
}
