//! LeanString hot paths: in-development version vs. last release (`lean_string_prev`), std as floor.

use std::hint::black_box;

use bench::{STATIC_STR_16, STATIC_STR_40, ascii, differ_at_last, samples};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use duplicate::duplicate;

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
                label         krate;
                ["current"]   [lean_string];
                ["prev"]      [lean_string_prev];
            ]
            group.bench_with_input(BenchmarkId::new(label, kind), &kind, |b, _| {
                b.iter(|| krate::LeanString::from_static_str(black_box(s)))
            });
        }
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
    const CHUNK: &str = "abcdefgh";
    const N: usize = 16;
    duplicate! {
        [
            label         empty;
            ["current"]   [lean_string::LeanString::new()];
            ["prev"]      [lean_string_prev::LeanString::new()];
            ["std"]       [String::new()];
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

fn push_str_after_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_str_after_clone");
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
                let one = black_box(StrTy::from(s.as_str()));
                let two = black_box(one.clone());
                group.bench_with_input(BenchmarkId::new(format!("{}/cloned", &label), len), &len, |b, _| {
                    b.iter(|| black_box(&one) == black_box(&two) )
                });
            }
        }
    }
    group.finish();
}

fn ne(c: &mut Criterion) {
    let mut group = c.benchmark_group("ne");
    for s in samples() {
        if s.is_empty() {
            continue;
        }
        let len = s.len();
        let other = differ_at_last(&s);
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            {
                let lhs = black_box(StrTy::from(s.as_str()));
                let rhs = black_box(StrTy::from(other.as_str()));
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(&lhs) == black_box(&rhs) )
                });
                let one = black_box(StrTy::from(s.as_str()));
                let two = {
                    let mut x = black_box(one.clone());
                    x.pop();
                    x
                };
                group.bench_with_input(BenchmarkId::new(format!("{}/cloned", &label), len), &len, |b, _| {
                    b.iter(|| black_box(&one) == black_box(&two) )
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
    ne
);
criterion_main!(apis);
