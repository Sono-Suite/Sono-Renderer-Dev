use anyhow::{bail, Context, Result};
use renderer::{runtime::WatchVm, watch::EngineNode};
use std::{
    collections::HashSet,
    env,
    ffi::{c_char, c_void, CString},
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

type NativeRun = unsafe extern "C" fn(*const f64, usize) -> f64;
const MEMORY_BLOCK: i64 = 7300;
const MAX_EXPANDED_OPERATIONS: usize = 100_000;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(filename: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}

#[derive(Clone)]
struct FunctionSpec {
    name: String,
    root: usize,
}

struct Module {
    handle: *mut c_void,
    functions: Vec<NativeRun>,
}

impl Module {
    fn load(path: &Path, functions: &[FunctionSpec]) -> Result<Self> {
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe { LoadLibraryW(wide_path.as_ptr()) };
        if handle.is_null() {
            bail!("LoadLibraryW failed for {}", path.display());
        }
        let mut entries = Vec::with_capacity(functions.len());
        for index in 0..functions.len() {
            let name = CString::new(format!("sono_run_{index}"))?;
            let address = unsafe { GetProcAddress(handle, name.as_ptr()) };
            if address.is_null() {
                unsafe { FreeLibrary(handle) };
                bail!("native module is missing {}", name.to_string_lossy());
            }
            entries.push(unsafe { std::mem::transmute::<*mut c_void, NativeRun>(address) });
        }
        Ok(Self {
            handle,
            functions: entries,
        })
    }

    fn run(&self, index: usize, inputs: &[f64]) -> f64 {
        unsafe { (self.functions[index])(inputs.as_ptr(), inputs.len()) }
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { FreeLibrary(self.handle) };
        }
    }
}

fn node(value: Option<f64>, func: Option<&str>, args: &[usize]) -> EngineNode {
    EngineNode {
        func: func.map(str::to_owned),
        args: args
            .iter()
            .map(|index| serde_json::Value::from(*index))
            .collect(),
        value: value.map(serde_json::Value::from),
        extra: Default::default(),
    }
}

fn constant(nodes: &mut Vec<EngineNode>, value: f64) -> usize {
    let index = nodes.len();
    nodes.push(node(Some(value), None, &[]));
    index
}

fn operation(nodes: &mut Vec<EngineNode>, name: &str, args: &[usize]) -> usize {
    let index = nodes.len();
    nodes.push(node(None, Some(name), args));
    index
}

struct Codegen<'a> {
    nodes: &'a [EngineNode],
    cuts: Vec<usize>,
    active: HashSet<usize>,
    expanded: usize,
}

impl<'a> Codegen<'a> {
    fn expression(&mut self, index: usize) -> Result<String> {
        if let Some(input) = self.cuts.iter().position(|candidate| *candidate == index) {
            return Ok(format!("inputs[{input}]"));
        }
        self.expanded += 1;
        if self.expanded > MAX_EXPANDED_OPERATIONS {
            bail!("expression expansion exceeded {MAX_EXPANDED_OPERATIONS} nodes");
        }
        if !self.active.insert(index) {
            bail!("cyclic Watch graph at node {index}");
        }
        let result = (|| {
            let node = self
                .nodes
                .get(index)
                .with_context(|| format!("Watch node index {index} is out of bounds"))?;
            if let Some(value) = node.value.as_ref().and_then(serde_json::Value::as_f64) {
                return Ok(format!("f64::from_bits(0x{:016x})", value.to_bits()));
            }
            let name = node
                .func
                .as_deref()
                .with_context(|| format!("Watch node {index} has no numeric value or function"))?;
            let children = node
                .args
                .iter()
                .map(|arg| {
                    arg.as_u64()
                        .map(|value| value as usize)
                        .with_context(|| format!("invalid child index in Watch node {index}"))
                })
                .collect::<Result<Vec<_>>>()?;
            self.function(name, &children)
                .with_context(|| format!("compiling Watch function {name} at node {index}"))
        })();
        self.active.remove(&index);
        result
    }

    fn function(&mut self, name: &str, children: &[usize]) -> Result<String> {
        let arg = |this: &mut Self, position: usize| -> Result<String> {
            let child = *children
                .get(position)
                .with_context(|| format!("missing argument {position}"))?;
            this.expression(child)
        };
        let variadic_fold = |this: &mut Self,
                             children: &[usize],
                             initial: Option<&str>,
                             method: &str|
         -> Result<String> {
            let first = *children
                .first()
                .context("variadic function requires an argument")?;
            let mut result = if let Some(initial) = initial {
                format!("({initial} + {})", this.expression(first)?)
            } else {
                this.expression(first)?
            };
            for child in children.iter().skip(1) {
                result = match method {
                    "+" | "*" | "-" | "/" | "%" => {
                        format!("({result} {method} {})", this.expression(*child)?)
                    }
                    "powf" => format!("({result}).powf({})", this.expression(*child)?),
                    "min" | "max" => {
                        format!("({result}).{method}({})", this.expression(*child)?)
                    }
                    _ => unreachable!(),
                };
            }
            Ok(result)
        };

        match name {
            "Add" => variadic_fold(self, children, Some("0.0_f64"), "+"),
            "Multiply" => {
                let first = *children
                    .first()
                    .context("variadic function requires an argument")?;
                let mut result = format!("(1.0_f64 * {})", self.expression(first)?);
                for child in children.iter().skip(1) {
                    result = format!("({result} * {})", self.expression(*child)?);
                }
                Ok(result)
            }
            "Min" => variadic_fold(self, children, None, "min"),
            "Max" => variadic_fold(self, children, None, "max"),
            "Rem" => variadic_fold(self, children, None, "%"),
            "Subtract" => variadic_fold(self, children, None, "-"),
            "Divide" => variadic_fold(self, children, None, "/"),
            "Power" => variadic_fold(self, children, None, "powf"),
            "Equal" | "NotEqual" | "Greater" | "GreaterOr" | "Less" | "LessOr" => {
                if children.len() < 2 {
                    bail!("comparison requires at least two arguments");
                }
                let left = arg(self, 0)?;
                let right = arg(self, 1)?;
                let operator = match name {
                    "Equal" => "==",
                    "NotEqual" => "!=",
                    "Greater" => ">",
                    "GreaterOr" => ">=",
                    "Less" => "<",
                    "LessOr" => "<=",
                    _ => unreachable!(),
                };
                Ok(format!(
                    "if {left} {operator} {right} {{ 1.0_f64 }} else {{ 0.0_f64 }}"
                ))
            }
            "Arctan2" => {
                if children.len() != 2 {
                    bail!("Arctan2 requires exactly two arguments");
                }
                Ok(format!("({}).atan2({})", arg(self, 0)?, arg(self, 1)?))
            }
            "If" => {
                if children.len() < 3 {
                    bail!("If requires condition, true branch, and false branch");
                }
                let condition = arg(self, 0)?;
                let when_zero = arg(self, 2)?;
                let when_nonzero = arg(self, 1)?;
                Ok(format!("{{ let condition = {condition}; if condition == 0.0 {{ {when_zero} }} else {{ {when_nonzero} }} }}"))
            }
            "And" | "Or" => {
                if children.is_empty() {
                    bail!("{name} requires at least one argument");
                }
                let mut remaining = children.iter().rev();
                let mut result = if name == "And" {
                    let value = self.expression(*remaining.next().unwrap())?;
                    format!(
                        "{{ let value = {value}; if value == 0.0 {{ 0.0_f64 }} else {{ value }} }}"
                    )
                } else {
                    "0.0_f64".to_owned()
                };
                for child in remaining {
                    let value = self.expression(*child)?;
                    result = if name == "And" {
                        format!("{{ let value = {value}; if value == 0.0 {{ 0.0_f64 }} else {{ {result} }} }}")
                    } else {
                        format!("{{ let value = {value}; if value != 0.0 {{ value }} else {{ {result} }} }}")
                    };
                }
                Ok(result)
            }
            "Abs" | "Arctan" | "Ceil" | "Cos" | "Floor" | "Log" | "Negate" | "Not" | "Round"
            | "Sin" | "Trunc" | "Frac" | "Sign" | "Radian" | "Degree" | "Arcsin" | "Arccos"
            | "Tan" | "Cosh" | "Sinh" | "Tanh" => {
                let value = arg(self, 0)?;
                Ok(match name {
                    "Abs" => format!("({value}).abs()"),
                    "Arctan" => format!("({value}).atan()"),
                    "Ceil" => format!("({value}).ceil()"),
                    "Cos" => format!("({value}).cos()"),
                    "Floor" => format!("({value}).floor()"),
                    "Log" => format!("({value}).ln()"),
                    "Negate" => format!("-({value})"),
                    "Not" => format!("if ({value}) == 0.0 {{ 1.0_f64 }} else {{ 0.0_f64 }}"),
                    "Round" => format!("({value}).round()"),
                    "Sin" => format!("({value}).sin()"),
                    "Trunc" => format!("({value}).trunc()"),
                    "Frac" => format!("{{ let value = {value}; value - value.trunc() }}"),
                    "Sign" => format!("({value}).signum()"),
                    "Radian" => format!("({value}).to_radians()"),
                    "Degree" => format!("({value}).to_degrees()"),
                    "Arcsin" => format!("({value}).asin()"),
                    "Arccos" => format!("({value}).acos()"),
                    "Tan" => format!("({value}).tan()"),
                    "Cosh" => format!("({value}).cosh()"),
                    "Sinh" => format!("({value}).sinh()"),
                    "Tanh" => format!("({value}).tanh()"),
                    _ => unreachable!(),
                })
            }
            _ => bail!("unsupported Watch operation {name:?}"),
        }
    }
}

fn compile_source(
    nodes: &[EngineNode],
    functions: &[FunctionSpec],
    cut_nodes: &[usize],
) -> Result<String> {
    if cut_nodes.iter().any(|index| *index >= nodes.len()) {
        bail!("a cut node index is out of bounds");
    }
    let mut source = String::from("#![allow(unused_variables, unused_parens)]\n");
    for (index, function) in functions.iter().enumerate() {
        let mut generator = Codegen {
            nodes,
            cuts: cut_nodes.to_vec(),
            active: HashSet::new(),
            expanded: 0,
        };
        let expression = generator.expression(function.root)?;
        source.push_str(&format!(
            "#[no_mangle]\npub unsafe extern \"C\" fn sono_run_{index}(input_ptr: *const f64, input_len: usize) -> f64 {{\n\
             if input_ptr.is_null() || input_len != {} {{ return f64::NAN; }}\n\
             let inputs = std::slice::from_raw_parts(input_ptr, input_len);\n\
             {expression}\n}}\n",
            cut_nodes.len()
        ));
    }
    Ok(source)
}

fn compile_and_load(
    nodes: &[EngineNode],
    functions: &[FunctionSpec],
    cut_nodes: &[usize],
    output_dir: &Path,
) -> Result<(Module, Duration, usize)> {
    fs::create_dir_all(output_dir)?;
    let source = compile_source(nodes, functions, cut_nodes)?;
    let source_bytes = source.len();
    let source_path = output_dir.join("sono_gcc_generated.rs");
    let dll_path = output_dir.join("sono_gcc_generated.dll");
    fs::write(&source_path, source)?;
    let compile_start = Instant::now();
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = Command::new(rustc)
        .arg("--edition=2021")
        .arg("--crate-type=cdylib")
        .arg("-Copt-level=3")
        .arg("-Awarnings")
        .arg(&source_path)
        .arg("-o")
        .arg(&dll_path)
        .status()
        .context("starting rustc for generated host code")?;
    if !status.success() {
        bail!("rustc failed to compile the generated module ({status})");
    }
    let module = Module::load(&dll_path, functions)?;
    Ok((module, compile_start.elapsed(), source_bytes))
}

fn build_workload() -> (Vec<EngineNode>, Vec<FunctionSpec>, Vec<usize>) {
    let mut nodes = vec![
        node(Some(MEMORY_BLOCK as f64), None, &[]),
        node(Some(0.0), None, &[]),
        node(None, Some("Get"), &[0, 1]),
        node(Some(1.0), None, &[]),
        node(None, Some("Get"), &[0, 3]),
        node(Some(2.0), None, &[]),
        node(None, Some("Get"), &[0, 5]),
    ];
    let x = 2;
    let y = 4;
    let z = 6;
    let mut functions = Vec::new();
    let mut add_fn = |name: &str, root: usize| {
        functions.push(FunctionSpec {
            name: name.into(),
            root,
        })
    };
    for name in [
        "Add", "Multiply", "Subtract", "Divide", "Power", "Min", "Max", "Rem",
    ] {
        let root = operation(&mut nodes, name, &[x, y, z]);
        add_fn(&format!("{name}_3"), root);
    }
    for name in [
        "Equal",
        "NotEqual",
        "Greater",
        "GreaterOr",
        "Less",
        "LessOr",
    ] {
        let root = operation(&mut nodes, name, &[x, y]);
        add_fn(name, root);
    }
    for name in [
        "Abs", "Arctan", "Ceil", "Cos", "Floor", "Log", "Negate", "Not", "Round", "Sin", "Trunc",
        "Frac", "Sign", "Radian", "Degree", "Arcsin", "Arccos", "Tan", "Cosh", "Sinh", "Tanh",
    ] {
        let root = operation(&mut nodes, name, &[x]);
        add_fn(name, root);
    }
    let root = operation(&mut nodes, "Arctan2", &[x, y]);
    add_fn("Arctan2", root);
    let root = operation(&mut nodes, "If", &[x, y, z]);
    add_fn("If", root);
    let root = operation(&mut nodes, "And", &[x, y, z]);
    add_fn("And", root);
    let root = operation(&mut nodes, "Or", &[x, y, z]);
    add_fn("Or", root);

    for size in [8, 16, 32] {
        let mut value = x;
        for _ in 0..size {
            let sum = operation(&mut nodes, "Add", &[value, y]);
            let difference = operation(&mut nodes, "Subtract", &[sum, z]);
            let half = constant(&mut nodes, 0.5);
            value = operation(&mut nodes, "Multiply", &[difference, half]);
        }
        add_fn(&format!("chain_{size}"), value);
    }
    (nodes, functions, vec![x, y, z])
}

fn exact_equal(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

fn differential(
    nodes: &[EngineNode],
    functions: &[FunctionSpec],
    module: &Module,
) -> Result<usize> {
    let edge_values = [
        f64::NEG_INFINITY,
        -1.0,
        -0.0,
        0.0,
        0.25,
        1.0,
        2.0,
        f64::INFINITY,
        f64::NAN,
    ];
    let mut comparisons = 0;
    let mut vm = WatchVm::new(nodes);
    for (left_index, left) in edge_values.iter().enumerate() {
        for (right_index, right) in edge_values.iter().enumerate() {
            let third = edge_values[(left_index * 3 + right_index) % edge_values.len()];
            vm.memory.set(MEMORY_BLOCK, 0, *left);
            vm.memory.set(MEMORY_BLOCK, 1, *right);
            vm.memory.set(MEMORY_BLOCK, 2, third);
            let inputs = [
                vm.memory.get(MEMORY_BLOCK, 0),
                vm.memory.get(MEMORY_BLOCK, 1),
                vm.memory.get(MEMORY_BLOCK, 2),
            ];
            for (function_index, function) in functions.iter().enumerate() {
                let interpreted = vm.execute(function.root).with_context(|| {
                    format!(
                        "interpreter {} at x={left}, y={right}, z={third}",
                        function.name
                    )
                })?;
                let compiled = module.run(function_index, &inputs);
                if !exact_equal(interpreted, compiled) {
                    bail!(
                        "differential mismatch in {} at x={left}, y={right}, z={third}: interpreter={interpreted:?} (0x{:016x}), compiled={compiled:?} (0x{:016x})",
                        function.name,
                        interpreted.to_bits(),
                        compiled.to_bits()
                    );
                }
                comparisons += 1;
            }
        }
    }
    Ok(comparisons)
}

fn sample_inputs(iteration: usize) -> [f64; 3] {
    let x = ((iteration % 101) as f64 - 50.0) * 0.125;
    let y = ((iteration % 37) as f64 - 18.0) * 0.0625;
    let z = (iteration % 11) as f64 * 0.25 + 0.5;
    [x, y, z]
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn bench_interpreter(nodes: &[EngineNode], root: usize, iterations: usize) -> Result<Duration> {
    let mut vm = WatchVm::new(nodes);
    let start = Instant::now();
    let mut checksum = 0_u64;
    for iteration in 0..iterations {
        let inputs = sample_inputs(iteration);
        for (slot, value) in inputs.iter().enumerate() {
            vm.memory.set(MEMORY_BLOCK, slot, *value);
        }
        let result = vm.execute(root)?;
        checksum = checksum.rotate_left(3) ^ result.to_bits();
    }
    std::hint::black_box(checksum);
    Ok(start.elapsed())
}

fn bench_native(module: &Module, function_index: usize, iterations: usize) -> Duration {
    let mut memory = renderer::runtime::Memory::new();
    let start = Instant::now();
    let mut checksum = 0_u64;
    for iteration in 0..iterations {
        let values = sample_inputs(iteration);
        for (slot, value) in values.iter().enumerate() {
            memory.set(MEMORY_BLOCK, slot, *value);
        }
        let inputs: [f64; 3] = std::array::from_fn(|slot| memory.get(MEMORY_BLOCK, slot));
        let result = module.run(function_index, std::hint::black_box(&inputs));
        checksum = checksum.rotate_left(3) ^ result.to_bits();
    }
    std::hint::black_box(checksum);
    start.elapsed()
}

fn duration_per_call_ns(duration: Duration, count: usize) -> f64 {
    duration.as_secs_f64() * 1.0e9 / count as f64
}

fn main() -> Result<()> {
    let output_dir = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/sono-gcc-feasibility/native"));
    let (nodes, functions, cuts) = build_workload();
    println!(
        "nodes={} functions={} cut_memory_reads={cuts:?}",
        nodes.len(),
        functions.len()
    );
    let (module, compile_time, generated_bytes) =
        compile_and_load(&nodes, &functions, &cuts, &output_dir)?;
    println!(
        "generated_source_bytes={generated_bytes} native_compile_and_load_ms={:.3}",
        compile_time.as_secs_f64() * 1000.0
    );
    let comparisons = differential(&nodes, &functions, &module)?;
    println!("differential_comparisons={comparisons} bitwise_mismatches=0");

    let iterations = env::var("SONO_GCC_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000usize);
    let repetitions = 5usize;
    println!("benchmark_iterations={iterations} repetitions={repetitions} mode=release");
    for size in [8, 16, 32] {
        let (function_index, spec) = functions
            .iter()
            .enumerate()
            .find(|(_, function)| function.name == format!("chain_{size}"))
            .context("missing chain benchmark")?;
        let mut interpreter_times = Vec::new();
        let mut native_times = Vec::new();
        for _ in 0..repetitions {
            interpreter_times.push(bench_interpreter(&nodes, spec.root, iterations)?);
            native_times.push(bench_native(&module, function_index, iterations));
        }
        let interpreter = median(interpreter_times);
        let native = median(native_times);
        let interpreter_ns = duration_per_call_ns(interpreter, iterations);
        let native_ns = duration_per_call_ns(native, iterations);
        println!(
            "chain_steps={size} vm_median_ns_per_call={interpreter_ns:.1} native_plus_memory_median_ns_per_call={native_ns:.1} vm_over_compiled_ratio={:.2}x",
            interpreter_ns / native_ns
        );
    }
    println!("note=kernel_only; no WatchRuntime lifecycle, event generation, rendering, GPU, readback, or export timing");
    Ok(())
}
