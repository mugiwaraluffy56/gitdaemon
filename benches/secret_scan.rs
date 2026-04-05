use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use gitdaemon::git::secrets::{scan_diff, scan_line};

// ── fixtures ─────────────────────────────────────────────────────────────────

const CLEAN_LINE: &str = "+    let config = Config::load(path)?;";
const AWS_LINE: &str = "+    key = \"AKIAIOSFODNN7EXAMPLE\"";
const GITHUB_LINE: &str = "+TOKEN=ghp_abcdefghijklmnopqrstuvwxyz123456789";

fn small_clean_diff() -> String {
    (0..20)
        .map(|i| format!("+    let x{} = {};\n", i, i))
        .collect()
}

fn large_clean_diff() -> String {
    (0..500)
        .map(|i| format!("+    let x{} = {};\n", i, i))
        .collect()
}

fn diff_with_secret_at_end() -> String {
    let mut d = large_clean_diff();
    d.push_str("+    api_key = \"AKIAIOSFODNN7EXAMPLE\"\n");
    d
}

// ── benchmarks ───────────────────────────────────────────────────────────────

fn bench_scan_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan_line");

    group.bench_function("clean", |b| {
        b.iter(|| scan_line(black_box(CLEAN_LINE)))
    });
    group.bench_function("aws_key", |b| {
        b.iter(|| scan_line(black_box(AWS_LINE)))
    });
    group.bench_function("github_pat", |b| {
        b.iter(|| scan_line(black_box(GITHUB_LINE)))
    });

    group.finish();
}

fn bench_scan_diff(c: &mut Criterion) {
    let small = small_clean_diff();
    let large = large_clean_diff();
    let with_secret = diff_with_secret_at_end();

    let mut group = c.benchmark_group("scan_diff");

    for (label, diff) in [
        ("small_clean", &small),
        ("large_clean", &large),
        ("large_with_secret_at_end", &with_secret),
    ] {
        group.bench_with_input(BenchmarkId::new("lines", label), diff, |b, d| {
            b.iter(|| scan_diff(black_box(d)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_scan_line, bench_scan_diff);
criterion_main!(benches);
