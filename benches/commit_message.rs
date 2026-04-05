use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use git2::Delta;
use gitdaemon::git::commit::{build_summary_pub, parse_symbol_pub};
use std::path::PathBuf;

// ── fixtures ─────────────────────────────────────────────────────────────────

fn small_deltas() -> Vec<(Delta, PathBuf)> {
    vec![
        (Delta::Modified, PathBuf::from("src/git/commit.rs")),
        (Delta::Added, PathBuf::from("src/git/ai_commit.rs")),
    ]
}

fn large_deltas() -> Vec<(Delta, PathBuf)> {
    let mut v = Vec::new();
    for i in 0..20 {
        v.push((Delta::Modified, PathBuf::from(format!("src/git/module{}.rs", i))));
    }
    for i in 0..5 {
        v.push((Delta::Added, PathBuf::from(format!("src/daemon/handler{}.rs", i))));
    }
    for i in 0..3 {
        v.push((Delta::Deleted, PathBuf::from(format!("src/old/legacy{}.rs", i))));
    }
    v
}

fn rust_diff_lines() -> Vec<String> {
    vec![
        "+pub struct PushQueue { commits: Vec<Commit> }".to_string(),
        "+pub async fn try_push(repo: &GitRepo) -> Result<()> {".to_string(),
        "+pub enum PushState { Idle, Pushing, Paused }".to_string(),
        "+    fn push_inner(&self) -> Result<()> {".to_string(),
        "-fn old_push() {}".to_string(),
        "+pub trait Pushable { fn push(&self) -> Result<()>; }".to_string(),
    ]
}

// ── benchmarks ───────────────────────────────────────────────────────────────

fn bench_build_summary(c: &mut Criterion) {
    let small = small_deltas();
    let large = large_deltas();
    let empty_syms = vec![];

    // Parse symbols from a typical Rust diff
    let symbols: Vec<_> = rust_diff_lines()
        .iter()
        .filter_map(|line| {
            let added = line.starts_with('+');
            parse_symbol_pub(line, added)
        })
        .collect();

    let mut group = c.benchmark_group("build_summary");

    group.bench_with_input(
        BenchmarkId::new("small_no_symbols", "2 files"),
        &(&small, &empty_syms),
        |b, (d, s)| b.iter(|| build_summary_pub(black_box(d), black_box(s))),
    );

    group.bench_with_input(
        BenchmarkId::new("large_no_symbols", "28 files"),
        &(&large, &empty_syms),
        |b, (d, s)| b.iter(|| build_summary_pub(black_box(d), black_box(s))),
    );

    group.bench_with_input(
        BenchmarkId::new("small_with_symbols", "2 files + 5 symbols"),
        &(&small, &symbols),
        |b, (d, s)| b.iter(|| build_summary_pub(black_box(d), black_box(s))),
    );

    group.bench_with_input(
        BenchmarkId::new("large_with_symbols", "28 files + 5 symbols"),
        &(&large, &symbols),
        |b, (d, s)| b.iter(|| build_summary_pub(black_box(d), black_box(s))),
    );

    group.finish();
}

fn bench_parse_symbol(c: &mut Criterion) {
    let lines = rust_diff_lines();

    c.bench_function("parse_symbol_from_diff_lines", |b| {
        b.iter(|| {
            for line in &lines {
                let added = line.starts_with('+');
                let _ = parse_symbol_pub(black_box(line), added);
            }
        })
    });
}

criterion_group!(benches, bench_build_summary, bench_parse_symbol);
criterion_main!(benches);
