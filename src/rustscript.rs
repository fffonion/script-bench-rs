use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use rand::{rngs::StdRng, RngExt, SeedableRng};
use vm::{
    compile_source_with_flavor, CallOutcome, CallReturn, HostFunctionRegistry, JitConfig,
    SourceFlavor, Value, Vm, VmStatus,
};

#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RustData(pub String);

thread_local! {
    static BENCH_RNG: RefCell<StdRng> = RefCell::new(StdRng::seed_from_u64(0));
}

static RAND_CALLS: AtomicU64 = AtomicU64::new(0);
static DATA_NEW_CALLS: AtomicU64 = AtomicU64::new(0);
static DATA_LT_CALLS: AtomicU64 = AtomicU64::new(0);
static RAND_NS: AtomicU64 = AtomicU64::new(0);
static DATA_NEW_NS: AtomicU64 = AtomicU64::new(0);
static DATA_LT_NS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub enum ExecMode {
    Interpreter,
    Jit,
    Aot,
}

impl ExecMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interpreter => "rustscript-interpreter",
            Self::Jit => "rustscript-jit",
            Self::Aot => "rustscript-aot",
        }
    }
}

pub fn sort_userdata(
    mode: ExecMode,
    run: impl FnOnce(&mut dyn FnMut()),
    validate: impl FnOnce(Vec<RustData>),
) -> anyhow::Result<()> {
    sort_userdata_with_len(mode, 10_000, run, validate)
}

pub fn sort_userdata_with_len(
    mode: ExecMode,
    item_count: usize,
    run: impl FnOnce(&mut dyn FnMut()),
    validate: impl FnOnce(Vec<RustData>),
) -> anyhow::Result<()> {
    let source = sort_userdata_source(item_count);
    let compiled = compile_source_with_flavor(&source, SourceFlavor::RustScript)?;
    let program = compiled.program.with_local_count(compiled.locals);
    let registry = rustscript_registry();

    let mut vm = new_vm(&program, &registry, mode)?;
    let dump_bridge = std::env::var_os("RUSTSCRIPT_DUMP_BRIDGE").is_some();
    if dump_bridge {
        vm.set_jit_native_bridge_stats_enabled(true);
    }
    reset_bench_rng();
    reset_host_metrics();
    let status = vm.run()?;
    ensure_halted(status)?;
    validate(stack_to_rust_data(vm.stack())?);
    if dump_bridge {
        eprintln!("warm bridge={:?}", vm.jit_native_bridge_stats_snapshot());
        eprintln!("aot_info=\n{}", vm.dump_aot_info());
        vm.clear_jit_native_bridge_stats();
    }

    run(&mut || {
        reset_bench_rng();
        reset_host_metrics();
        vm.reset_for_reuse()
            .expect("rustscript benchmark VM should reset");
        let status = vm.run().expect("rustscript benchmark VM should run");
        ensure_halted(status).expect("rustscript benchmark VM should halt");
    });
    if dump_bridge {
        eprintln!("host_metrics={}", host_metrics());
        eprintln!("run bridge={:?}", vm.jit_native_bridge_stats_snapshot());
        eprintln!("aot_info=\n{}", vm.dump_aot_info());
    }

    Ok(())
}

fn sort_userdata_source(item_count: usize) -> String {
    include_str!("../scripts/sort_userdata.rss")
        .replace("while i < 10000", &format!("while i < {item_count}"))
}

fn new_vm(
    program: &vm::Program,
    registry: &HostFunctionRegistry,
    mode: ExecMode,
) -> anyhow::Result<Vm> {
    let mut vm = Vm::new(program.clone());
    registry.bind_vm_cached(&mut vm)?;
    vm.set_jit_config(JitConfig {
        enabled: matches!(mode, ExecMode::Jit),
        hot_loop_threshold: 1,
        max_trace_len: 16_384,
    });
    if matches!(mode, ExecMode::Aot) {
        vm.compile_aot()?;
    }
    Ok(vm)
}

fn rustscript_registry() -> HostFunctionRegistry {
    let mut registry = HostFunctionRegistry::new();
    registry.register_static_non_yielding_args("rand", 1, host_rand);
    registry.register_static_non_yielding_args("data_new", 1, host_data_new);
    registry.register_static_non_yielding_args("data_lt", 2, host_data_lt);
    registry
}

fn host_rand(args: &[Value]) -> vm::VmResult<CallOutcome> {
    let started = Instant::now();
    let [Value::Int(limit)] = args else {
        return Err(vm::VmError::TypeMismatch("rand(int)"));
    };
    if *limit <= 0 {
        return Err(vm::VmError::HostError(
            "rand limit must be positive".to_string(),
        ));
    }
    let value = BENCH_RNG.with(|rng| rng.borrow_mut().random_range(0..*limit));
    RAND_CALLS.fetch_add(1, Ordering::Relaxed);
    RAND_NS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    Ok(CallOutcome::Return(CallReturn::one(Value::Int(value))))
}

fn reset_bench_rng() {
    BENCH_RNG.with(|rng| *rng.borrow_mut() = StdRng::seed_from_u64(0));
}

fn reset_host_metrics() {
    for metric in [
        &RAND_CALLS,
        &DATA_NEW_CALLS,
        &DATA_LT_CALLS,
        &RAND_NS,
        &DATA_NEW_NS,
        &DATA_LT_NS,
    ] {
        metric.store(0, Ordering::Relaxed);
    }
}

fn host_metrics() -> String {
    format!(
        "rand={}/{}us data_new={}/{}us data_lt={}/{}us",
        RAND_CALLS.load(Ordering::Relaxed),
        RAND_NS.load(Ordering::Relaxed) / 1000,
        DATA_NEW_CALLS.load(Ordering::Relaxed),
        DATA_NEW_NS.load(Ordering::Relaxed) / 1000,
        DATA_LT_CALLS.load(Ordering::Relaxed),
        DATA_LT_NS.load(Ordering::Relaxed) / 1000,
    )
}

fn host_data_new(args: &[Value]) -> vm::VmResult<CallOutcome> {
    let started = Instant::now();
    let [Value::String(text)] = args else {
        return Err(vm::VmError::TypeMismatch("data_new(string)"));
    };
    let result = CallOutcome::Return(CallReturn::one(Value::string(text.as_str())));
    DATA_NEW_CALLS.fetch_add(1, Ordering::Relaxed);
    DATA_NEW_NS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    Ok(result)
}

fn host_data_lt(args: &[Value]) -> vm::VmResult<CallOutcome> {
    let started = Instant::now();
    let [Value::String(left), Value::String(right)] = args else {
        return Err(vm::VmError::TypeMismatch("data_lt(string, string)"));
    };
    let result = CallOutcome::Return(CallReturn::one(Value::Bool(left.as_str() < right.as_str())));
    DATA_LT_CALLS.fetch_add(1, Ordering::Relaxed);
    DATA_LT_NS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    Ok(result)
}

fn ensure_halted(status: VmStatus) -> anyhow::Result<()> {
    if status != VmStatus::Halted {
        anyhow::bail!("expected halted VM, got {status:?}");
    }
    Ok(())
}

fn stack_to_rust_data(stack: &[Value]) -> anyhow::Result<Vec<RustData>> {
    let [Value::Array(values)] = stack else {
        anyhow::bail!("expected one array result, got {stack:?}");
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(text) => Ok(RustData(text.as_str().to_string())),
            other => anyhow::bail!("expected string item, got {other:?}"),
        })
        .collect()
}
