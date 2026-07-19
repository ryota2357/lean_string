//! Cross-library comparison against compact_str, ecow, std::String, and Arc<str>.

use bench::{STATIC_STR_16, STATIC_STR_40, ascii};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use duplicate::duplicate;

use compact_str::{CompactString, ToCompactString};
use ecow::EcoString;
use lean_string::{LeanString, ToLeanString};
use std::{borrow::Cow, hint::black_box, sync::Arc};

fn samples() -> Vec<String> {
    // 15: EcoString max inline
    // 16: LeanString max inline
    // 24: CompactString max inline
    [0, 1, 15, 16, 24, 25, 256].iter().map(|&n| ascii(n)).collect()
}

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
        group.bench_with_input(BenchmarkId::new("Cow<'static, str>", kind), &kind, |b, _| {
            b.iter(|| Cow::<'static, str>::from(black_box(s)))
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
                        s.push_str(black_box("abcdefgh"));
                        s
                    },
                    BatchSize::SmallInput,
                )
            });
            black_box(shared);
        }
    }
    group.finish();
}

fn grow(c: &mut Criterion) {
    let mut group = c.benchmark_group("Grow");
    duplicate! {
        [
            label              StrTy;
            ["LeanString"]     [LeanString];
            ["CompactString"]  [CompactString];
            ["EcoString"]      [EcoString];
            ["String"]         [String];
        ]
        group.bench_function(label, |b| {
            b.iter(|| {
                let mut s = StrTy::default();
                for _ in 0..black_box(16) {
                    s.push_str(black_box("abcdefgh"));
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
                    b.iter(|| uut.as_str())
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
                label              StrTy;
                ["LeanString"]     [LeanString];
                ["CompactString"]  [CompactString];
                ["EcoString"]      [EcoString];
                ["String"]         [String];
            ]
            {
                let lhs = black_box(StrTy::from(s.as_str()));
                let rhs = black_box(StrTy::from(s.as_str()));
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(&lhs) == black_box(&rhs))
                });
            }
        }
    }
    group.finish();
}

fn eq_cloned(c: &mut Criterion) {
    let mut group = c.benchmark_group("Eq/cloned");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label              StrTy;
                ["LeanString"]     [LeanString];
                ["EcoString"]      [EcoString];
            ]
            {
                let lhs = black_box(StrTy::from(s.as_str()));
                let rhs = black_box(lhs.clone());
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(&lhs) == black_box(&rhs))
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
    eq_cloned,
    construct_from_static,
    numbers
);
criterion_main!(comparison);
