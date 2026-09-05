//! Detection hot-path benchmark: the built-in pattern recognizer scanning
//! PII-dense text. The recognizer is built once; only `recognize` is timed.
//!
//! Two recognizers are compared, the bare pattern scan and the context-enhanced
//! one, so a regression can be attributed to the scan itself or to the boosting
//! layer. Throughput is reported in bytes scanned.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Recognizer, RecognizerContext, Scope};
use elide_pattern::PatternRecognizer;
use tokio::runtime::{Builder, Runtime};

/// One PII-dense paragraph, repeated to a realistic document size. Mixes
/// structured labels (email, phone, card, SSN, IP) with surrounding prose so the
/// scan does real work, not just match a bare token.
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

fn bench_scan(c: &mut Criterion) {
    let rt = runtime();
    let text = corpus();
    let data = TextData::new(text.clone());
    let scope = Scope::new();
    let ctx = RecognizerContext::new(&scope);

    let bare = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build()
        .expect("build recognizer");
    let enhanced = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()
        .expect("build context-enhanced recognizer");

    let mut group = c.benchmark_group("scan");
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("patterns", |b| {
        b.iter(|| {
            rt.block_on(bare.recognize(black_box(&data), &ctx))
                .expect("recognize")
        });
    });
    group.bench_function("context_enhanced", |b| {
        b.iter(|| {
            rt.block_on(Recognizer::<Text>::recognize(
                &enhanced,
                black_box(&data),
                &ctx,
            ))
            .expect("recognize")
        });
    });
    group.finish();
}

criterion_group!(benches, bench_scan);
criterion_main!(benches);
