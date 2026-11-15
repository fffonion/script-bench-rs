use std::time::Instant;

use script_bench::rustscript::{sort_userdata_with_len, ExecMode, RustData};

fn main() -> anyhow::Result<()> {
    let count = std::env::var("RUSTSCRIPT_SORT_COUNT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(100);
    let mode = match std::env::var("RUSTSCRIPT_MODE").as_deref() {
        Ok("jit") => ExecMode::Jit,
        Ok("aot") => ExecMode::Aot,
        _ => ExecMode::Interpreter,
    };
    let mut ran = false;
    let started = Instant::now();
    sort_userdata_with_len(
        mode,
        count,
        |func| {
            let inner = Instant::now();
            func();
            println!("run_us={}", inner.elapsed().as_micros());
            ran = true;
        },
        |array| {
            assert_eq!(array.len(), count);
            if std::env::var_os("RUSTSCRIPT_SKIP_SORT_ASSERT").is_some() {
                return;
            }
            let mut prev = RustData::default();
            for next in array {
                if prev > next {
                    panic!("order violation: prev={} next={}", prev.0, next.0);
                }
                prev = next;
            }
        },
    )?;
    println!(
        "mode={:?} count={count} total_us={} ran={ran}",
        mode,
        started.elapsed().as_micros()
    );
    Ok(())
}
