//! Redaction benchmarks: `pick` (selection) and `anonymize` (select + apply),
//! swept over entity count to surface the overlap-clustering's scaling.
//!
//! `pick` is sync; `anonymize` writes into the target, so each timed iteration
//! rebuilds fresh entities and a fresh `TextDoc` via `iter_batched`.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextDoc};
use elide_core::primitive::Confidence;
use elide_core::recognition::Scope;
use elide_operator::operators::Replace;
use elide_redaction::{Anonymizer, Rule};
use tokio::runtime::{Builder, Runtime};

/// A current-thread runtime (the `rt` feature; benches don't need multi-thread).
fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
}

/// Disjoint entities so they cluster one-per, isolating per-entity cost from the
/// merge path (overlap fusion is exercised separately below).
fn disjoint_entities(n: usize) -> Vec<Entity<Text>> {
    (0..n)
        .map(|i| {
            Entity::fixture_conf(
                "NAME",
                (i * 8, i * 8 + 5),
                Confidence::new(0.9).expect("valid confidence"),
            )
        })
        .collect()
}

/// A backing document long enough to cover every entity's span.
fn doc_for(n: usize) -> TextDoc {
    TextDoc::new("x".repeat(n * 8 + 8))
}

fn anonymizer() -> Anonymizer<Text> {
    Anonymizer::new().with(Rule::fallback(Replace::default()))
}

fn bench_pick(c: &mut Criterion) {
    let anonymizer = anonymizer();
    let scope = Scope::default();
    let mut group = c.benchmark_group("pick");
    for n in [10usize, 100, 1_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // pick mutates the audit trail, so rebuild entities each iteration.
            b.iter_batched_ref(
                || disjoint_entities(n),
                |entities| anonymizer.pick(black_box(entities), &scope),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_anonymize(c: &mut Criterion) {
    let rt = runtime();
    let anonymizer = anonymizer();
    let scope = Scope::default();
    let mut group = c.benchmark_group("anonymize");
    for n in [10usize, 100, 1_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || (disjoint_entities(n), doc_for(n)),
                |(mut entities, mut doc)| {
                    rt.block_on(anonymizer.anonymize(&mut doc, &mut entities, &scope))
                        .expect("anonymize");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_pick, bench_anonymize);
criterion_main!(benches);
