//! Cross-library comparison against compact_str, ecow, std::String, and Arc<str>.

use std::hint::black_box;
use std::sync::Arc;

use bench::{STATIC_STR_16, STATIC_STR_40, ascii, differ_at_last, samples};
use compact_str::{CompactString, ToCompactString};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use duplicate::duplicate;
use ecow::EcoString;
use lean_string::{LeanString, ToLeanString};

fn construct(c: &mut Criterion) {
    let mut group = c.benchmark_group("Construct");
    for s in samples() {
        let s = s.as_str();
        let len = s.len();
        duplicate! {
            [
                label              build;
                ["LeanString"]     [LeanString::from(black_box(s))];
                ["CompactString"]  [CompactString::from(black_box(s))];
                ["EcoString"]      [EcoString::from(black_box(s))];
                ["String"]         [String::from(black_box(s))];
                ["Arc<str>"]       [Arc::<str>::from(black_box(s))];
            ]
            group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                b.iter(|| build)
            });
        }
    }
    group.finish();
}

fn construct_from_static(c: &mut Criterion) {
    let mut group = c.benchmark_group("Construct from static str");
    for (kind, s) in [("short", STATIC_STR_16), ("long", STATIC_STR_40)] {
        group.bench_with_input(BenchmarkId::new("LeanString", kind), &kind, |b, _| {
            b.iter(|| LeanString::from_static_str(black_box(s)))
        });
        group.bench_with_input(BenchmarkId::new("CompactString", kind), &kind, |b, _| {
            b.iter(|| CompactString::const_new(black_box(s)))
        });
    }
    group.finish();
}

fn clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("Clone");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label            build;
                ["LeanString"]   [LeanString::from(s.as_str())];
                ["EcoString"]    [EcoString::from(s.as_str())];
                ["Arc<str>"]     [Arc::<str>::from(s.as_str())];
            ]
            {
                let uut = build;
                let uut = black_box(uut);
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| uut.clone())
                });
            }
        }
    }
    group.finish();
}

fn cow_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("CoW write");
    let base = ascii(64);
    duplicate! {
        [
            label            build;
            ["LeanString"]   [LeanString::from(base.as_str())];
            ["EcoString"]    [EcoString::from(base.as_str())];
        ]
        {
            let shared = build;
            group.bench_function(label, |b| {
                b.iter_batched(
                    || shared.clone(),
                    |mut s| {
                        s.push_str("!");
                        s
                    },
                    BatchSize::SmallInput,
                )
            });
        }
    }
    group.finish();
}

fn grow(c: &mut Criterion) {
    let mut group = c.benchmark_group("Grow");
    const CHUNK: &str = "abcdefgh";
    const N: usize = 16;
    duplicate! {
        [
            label              empty;
            ["LeanString"]     [LeanString::new()];
            ["CompactString"]  [CompactString::const_new("")];
            ["EcoString"]      [EcoString::new()];
            ["String"]         [String::new()];
        ]
        group.bench_function(label, |b| {
            b.iter(|| {
                let mut s = empty;
                for _ in 0..N {
                    s.push_str(black_box(CHUNK));
                }
                s
            })
        });
    }
    group.finish();
}

fn access(c: &mut Criterion) {
    let mut group = c.benchmark_group("Access");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label              build;
                ["LeanString"]     [LeanString::from(s.as_str())];
                ["CompactString"]  [CompactString::from(s.as_str())];
                ["EcoString"]      [EcoString::from(s.as_str())];
                ["String"]         [String::from(s.as_str())];
            ]
            {
                let uut = build;
                let uut = black_box(uut);
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(uut.as_str()))
                });
            }
        }
    }
    group.finish();
}

fn eq(c: &mut Criterion) {
    let mut group = c.benchmark_group("Eq");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label              build;
                ["LeanString"]     [LeanString::from(s.as_str())];
                ["CompactString"]  [CompactString::from(s.as_str())];
                ["EcoString"]      [EcoString::from(s.as_str())];
                ["String"]         [String::from(s.as_str())];
            ]
            {
                let lhs = build;
                let rhs = build;
                let lhs = black_box(lhs);
                let rhs = black_box(rhs);
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(lhs == rhs))
                });
            }
        }
    }
    group.finish();
}

fn ne(c: &mut Criterion) {
    let mut group = c.benchmark_group("Not eq");
    for s in samples() {
        if s.is_empty() {
            continue;
        }
        let len = s.len();
        let other = differ_at_last(&s);
        duplicate! {
            [
                label              StrTy;
                ["LeanString"]     [LeanString];
                ["CompactString"]  [CompactString];
                ["EcoString"]      [EcoString];
                ["String"]         [String];
            ]
            {
                let lhs = black_box(StrTy::from(s.as_str()));
                let rhs = black_box(StrTy::from(other.as_str()));
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(lhs != rhs))
                });
            }
        }
    }
    group.finish();
}

fn numbers(c: &mut Criterion) {
    let mut group = c.benchmark_group("Numbers");
    duplicate! {
        [
            name          value;
            ["u32"]       [1_234_567_u32];
            ["u64::MAX"]  [u64::MAX];
            ["i64"]       [-9_876_543_210_i64];
            ["f64"]       [12_345.678_9_f64];
        ]
        {
            group.bench_with_input(BenchmarkId::new("LeanString", name), &name, |b, _| {
                b.iter(|| ToLeanString::to_lean_string(black_box(&value)))
            });
            group.bench_with_input(BenchmarkId::new("CompactString", name), &name, |b, _| {
                b.iter(|| ToCompactString::to_compact_string(black_box(&value)))
            });
            group.bench_with_input(BenchmarkId::new("std", name), &name, |b, _| {
                b.iter(|| black_box(&value).to_string())
            });
        }
    }
    group.finish();
}

criterion_group!(
    comparison,
    construct,
    clone,
    cow_write,
    grow,
    access,
    eq,
    ne,
    construct_from_static,
    numbers
);
criterion_main!(comparison);
