use criterion::{criterion_group, criterion_main, Criterion};

use script_bench::rustscript::{ExecMode, RustData};

fn item_count() -> usize {
    std::env::var("RUSTSCRIPT_SORT_COUNT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(10_000)
}

fn selected_modes() -> Vec<ExecMode> {
    let raw = std::env::var("RUSTSCRIPT_BENCH_MODES")
        .unwrap_or_else(|_| "interpreter,jit,aot".to_string());
    raw.split(",")
        .filter_map(|mode| match mode.trim() {
            "interpreter" => Some(ExecMode::Interpreter),
            "jit" => Some(ExecMode::Jit),
            "aot" => Some(ExecMode::Aot),
            _ => None,
        })
        .collect()
}

fn bench_mode(c: &mut Criterion, mode: ExecMode, count: usize) {
    script_bench::rustscript::sort_userdata_with_len(
        mode,
        count,
        |func| {
            c.bench_function(mode.label(), |b| {
                b.iter(|| func());
            });
        },
        |array| {
            assert_eq!(array.len(), count);
            let mut prev = RustData::default();
            for next in array {
                assert!(prev <= next);
                prev = next;
            }
        },
    )
    .unwrap();
}

fn benchmark(c: &mut Criterion) {
    let count = item_count();
    for mode in selected_modes() {
        bench_mode(c, mode, count);
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = benchmark,
}

criterion_main!(benches);
