//! LeanString hot paths: in-development version vs. last release (`lean_string_prev`), std as floor.

use bench::{STATIC_STR_16, STATIC_STR_40, ascii};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use duplicate::duplicate;
use std::{borrow::Cow, hint::black_box};

fn samples() -> Vec<String> {
    [0, 1, 15, 16, 17, 256].iter().map(|&n| ascii(n)).collect()
}

fn from(c: &mut Criterion) {
    let mut group = c.benchmark_group("from");
    for s in samples() {
        let s = s.as_str();
        let len = s.len();
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                b.iter(|| StrTy::from(black_box(s)))
            });
        }
    }
    group.finish();
}

fn from_static_str(c: &mut Criterion) {
    let mut group = c.benchmark_group("from_static_str");
    for (kind, s) in [("short", STATIC_STR_16), ("long", STATIC_STR_40)] {
        duplicate! {
            [
                label         LeanString;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
            ]
            group.bench_with_input(BenchmarkId::new(label, kind), &kind, |b, _| {
                b.iter(|| LeanString::from_static_str(black_box(s)))
            });
        }
        group.bench_with_input(BenchmarkId::new("std", kind), &kind, |b, _| {
            b.iter(|| Cow::<'static, str>::from(black_box(s)))
        });
    }
    group.finish();
}

fn to_lean_string(c: &mut Criterion) {
    let mut group = c.benchmark_group("to_lean_string");
    duplicate! {
        [
            name          value;
            ["u32"]       [1_234_567_u32];
            ["u64::MAX"]  [u64::MAX];
            ["i64"]       [-9_876_543_210_i64];
            ["f64"]       [12_345.678_9_f64];
        ]
        {
            group.bench_with_input(BenchmarkId::new("current", name), &name, |b, _| {
                b.iter(|| lean_string::ToLeanString::to_lean_string(black_box(&value)))
            });
            group.bench_with_input(BenchmarkId::new("prev", name), &name, |b, _| {
                b.iter(|| lean_string_prev::ToLeanString::to_lean_string(black_box(&value)))
            });
            group.bench_with_input(BenchmarkId::new("std", name), &name, |b, _| {
                b.iter(|| black_box(&value).to_string())
            });
        }
    }
    group.finish();
}

fn clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            {
                let uut = black_box(StrTy::from(s.as_str()));
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| uut.clone())
                });
            }
        }
    }
    group.finish();
}

fn reserve(c: &mut Criterion) {
    let mut group = c.benchmark_group("reserve");
    for additional in [8usize, 64] {
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            group.bench_with_input(BenchmarkId::new(label, additional), &additional, |b, _| {
                b.iter(|| {
                    let mut s = StrTy::new();
                    s.reserve(black_box(additional));
                    s
                })
            });
        }
    }
    group.finish();
}

fn push_str(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_str");
    duplicate! {
        [
            label         StrTy;
            ["current"]   [lean_string::LeanString];
            ["prev"]      [lean_string_prev::LeanString];
            ["std"]       [String];
        ]
        group.bench_function(label, |b| {
            b.iter(|| {
                let mut s = StrTy::new();
                for _ in 0..black_box(16) {
                    s.push_str(black_box("abcdefgh"));
                }
                s
            })
        });
    }
    group.finish();
}

fn push_str_after_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_str/after_clone");
    let base = ascii(64);
    duplicate! {
        [
            label         StrTy;
            ["current"]   [lean_string::LeanString];
            ["prev"]      [lean_string_prev::LeanString];
            ["std"]       [String];
        ]
        {
            let shared = StrTy::from(base.as_str());
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

fn as_str(c: &mut Criterion) {
    let mut group = c.benchmark_group("as_str");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            {
                let uut = black_box(StrTy::from(s.as_str()));
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(uut.as_str()))
                });
            }
        }
    }
    group.finish();
}

fn eq(c: &mut Criterion) {
    let mut group = c.benchmark_group("eq");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            {
                let lhs = black_box(StrTy::from(s.as_str()));
                let rhs = black_box(StrTy::from(s.as_str()));
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(&lhs) == black_box(&rhs) )
                });
            }
        }
    }
    group.finish();
}

fn eq_cloned(c: &mut Criterion) {
    let mut group = c.benchmark_group("eq/cloned");
    for s in samples() {
        let len = s.len();
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            {
                let lhs = black_box(StrTy::from(s.as_str()));
                let rhs = black_box(lhs.clone());
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(&lhs) == black_box(&rhs) )
                });
            }
        }
    }
    group.finish();
}

criterion_group!(
    apis,
    from,
    from_static_str,
    to_lean_string,
    clone,
    reserve,
    push_str,
    push_str_after_clone,
    as_str,
    eq,
    eq_cloned,
);
criterion_main!(apis);
