//! End-to-end pipeline benchmark: `analyze` (detect) and `anonymize` (detect +
//! redact) over a PII-dense text document, through the public facade.
//!
//! The Orchestrator is built once; the timed closure decodes a fresh `Document`
//! each iteration (`anonymize` redacts it in place, so it cannot be reused) and
//! runs the phase. This is the whole-pipeline number, the first place a
//! regression anywhere in detect/reconcile/redact would show up. Throughput is
//! the document's byte size.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::{LabelCatalog, builtins};
use elide::modality::text::Text;
use elide::recognition::Scope;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Mask, Replace};
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Document, Orchestrator, RegistryDocumentExt};
use elide_office::docx::Docx;
use elide_office::opc::Replacement;
use tokio::runtime::{Builder, Runtime};

const PARAGRAPH: &str = "Contact alice.johnson@example.com or call +1 (628) 555-0175. \
Card 4111 1111 1111 1111 expires 09/27, SSN 123-45-6789, from 192.168.1.42. \
Wire to IBAN GB82 WEST 1234 5698 7654 32 before the invoice for $2,000,000.00 clears. ";

fn corpus() -> String {
    PARAGRAPH.repeat(16)
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
}

fn build_orchestrator() -> Orchestrator {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()
        .expect("build recognizer");
    let analyzer = Analyzer::<Text>::new().with_recognizer(patterns);
    let anonymizer = Anonymizer::<Text>::new()
        .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Mask::stars()))
        .with(Rule::fallback(Replace::default()));

    Orchestrator::new()
        .with_registry(FormatRegistry::with_builtin())
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_modality::<Text>(analyzer, anonymizer)
}

/// A big, PII-dense `.docx`: a real OPC package (~100 KB of decoded text) so the
/// bench exercises the container decode + part model + encode, not just plain
/// text. Committed under testdata so the bench has no synthesis code.
const LARGE_DOCX: &[u8] = include_bytes!("../tests/testdata/docx/large.docx");

/// Approximate decoded body-text size of `large.docx` (400 PII-dense paragraphs,
/// ~255 bytes each). The zipped file is ~1 KB; this is the size that drives the
/// pipeline's cost, so throughput is reported against it.
const DECODED_TEXT_LEN: u64 = 102_000;

/// Decode a fresh text document from the corpus (async: the codec runs on the rt).
async fn fresh_text(registry: &FormatRegistry, text: &str) -> Document {
    registry
        .document("sample.txt", text)
        .await
        .expect("decode text document")
}

/// Decode a fresh document from the big `.docx` bytes.
async fn fresh_docx(registry: &FormatRegistry) -> Document {
    registry
        .document("large.docx", LARGE_DOCX)
        .await
        .expect("decode docx document")
}

fn bench_pipeline(c: &mut Criterion) {
    let rt = runtime();
    let orchestrator = build_orchestrator();
    let registry = FormatRegistry::with_builtin();
    let text = corpus();

    let mut group = c.benchmark_group("pipeline");
    group.throughput(Throughput::Bytes(text.len() as u64));

    group.bench_function("analyze", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut doc = fresh_text(&registry, &text).await;
                orchestrator
                    .analyze(black_box(&mut doc), &Directives::new())
                    .await
                    .expect("analyze")
            })
        });
    });

    group.bench_function("anonymize", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut doc = fresh_text(&registry, &text).await;
                orchestrator
                    .anonymize(black_box(&mut doc), &Directives::new())
                    .await
                    .expect("anonymize")
            })
        });
    });

    group.finish();
}

/// The same pipeline over a big `.docx`, so decode/part-model/encode cost is
/// visible alongside the plain-text number. Throughput is the decoded-text size.
fn bench_docx(c: &mut Criterion) {
    let rt = runtime();
    let orchestrator = build_orchestrator();
    let registry = FormatRegistry::with_builtin();

    let mut group = c.benchmark_group("docx");
    // Decoded text length drives detection/redaction cost; the zipped file is
    // tiny (deflate crushes the repetition), so it is not the meaningful size.
    // `large.docx` carries ~100 KB of body text (see its generator note).
    group.throughput(Throughput::Bytes(DECODED_TEXT_LEN));
    // Each iteration is ~0.5s, so the default 100 samples take minutes; a
    // smaller sample keeps the suite CI-friendly while staying statistically
    // usable for regression tracking.
    group.sample_size(20);

    group.bench_function("analyze", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut doc = fresh_docx(&registry).await;
                orchestrator
                    .analyze(black_box(&mut doc), &Directives::new())
                    .await
                    .expect("analyze")
            })
        });
    });

    group.bench_function("anonymize", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut doc = fresh_docx(&registry).await;
                orchestrator
                    .anonymize(black_box(&mut doc), &Directives::new())
                    .await
                    .expect("anonymize")
            })
        });
    });

    group.finish();
}

/// Isolate the DOCX codec cost from detection: pure decode (open + extract text
/// blocks) and pure encode (rewrite every block back), calling the office engine
/// directly. This attributes the anonymize gap to decode / encode / detection.
fn bench_docx_codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("docx_codec");
    group.throughput(Throughput::Bytes(DECODED_TEXT_LEN));

    group.bench_function("decode", |b| {
        b.iter(|| {
            let docx = Docx::open(black_box(LARGE_DOCX)).expect("open");
            black_box(docx.extract())
        });
    });

    // Redact every extracted block (the maximal rewrite), so encode cost is
    // measured against a fully-edited body plus the untouched parts the raw-copy
    // path skips re-deflating.
    let docx = Docx::open(LARGE_DOCX).expect("open");
    let replacements: Vec<Replacement> = docx
        .extract()
        .blocks
        .iter()
        .map(|block| Replacement::for_block(block, "[REDACTED]"))
        .collect();
    group.bench_function("encode", |b| {
        b.iter(|| {
            black_box(docx.rewrite(black_box(&replacements)).expect("rewrite"));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_pipeline, bench_docx, bench_docx_codec);
criterion_main!(benches);
