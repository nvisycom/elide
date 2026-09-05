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

/// Decode a fresh document from the corpus (async: the codec runs on the rt).
async fn fresh_document(registry: &FormatRegistry, text: &str) -> Document {
    registry
        .document("sample.txt", text)
        .await
        .expect("decode document")
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
                let mut doc = fresh_document(&registry, &text).await;
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
                let mut doc = fresh_document(&registry, &text).await;
                orchestrator
                    .anonymize(black_box(&mut doc), &Directives::new())
                    .await
                    .expect("anonymize")
            })
        });
    });

    group.finish();
}

criterion_group!(benches, bench_pipeline);
criterion_main!(benches);
