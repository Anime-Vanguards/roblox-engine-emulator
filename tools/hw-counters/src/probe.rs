//! `ree-hw-counters probe`: checks that each counter counts what its name says, through the service, on kernels whose
//! counts are known: a loop of four instructions and one branch an iteration; random branches, half of which mispredict,
//! beside the same loop always taken; loads that hit the L1 data cache; and chases through a random cycle of cache lines
//! in buffers that fit the L1, the L2, the L3, and none (each step a load that waits on the one before). The thread is
//! pinned to one CPU, and each kernel runs between two readings, so its counts are exactly its own.

use std::arch::asm;

use crate::{client, ffi, protocol, readings, register};

/// 4 instructions and a branch an iteration
fn spin(iterations: u64) {
    unsafe {
        asm!("2:", "add {a}, 1", "add {b}, 1", "dec {n}", "jnz 2b", n = inout(reg) iterations => _, a = inout(reg) 0u64 => _, b = inout(reg) 0u64 => _, options(nomem, nostack));
    }
}

/// A xorshift step and a branch on `mask` of it an iteration (mask 1: taken at random; 0: always taken), and the
/// loop's own branch
fn branches(iterations: u64, mask: u64) {
    unsafe {
        asm!(
            "2:",
            "mov {t}, {x}", "shl {t}, 13", "xor {x}, {t}",
            "mov {t}, {x}", "shr {t}, 7", "xor {x}, {t}",
            "mov {t}, {x}", "shl {t}, 17", "xor {x}, {t}",
            "test {x}, {m}", "jz 3f", "add {c}, 1",
            "3:", "dec {n}", "jnz 2b",
            n = inout(reg) iterations => _, x = inout(reg) 0x9E37_79B9_7F4A_7C15u64 => _, t = out(reg) _, c = inout(reg) 0u64 => _, m = in(reg) mask,
            options(nomem, nostack)
        );
    }
}

/// Four loads from one cache line an iteration
fn loads(iterations: u64, line: *const u64) {
    unsafe {
        asm!(
            "2:", "mov {t}, qword ptr [{p}]", "mov {t}, qword ptr [{p} + 8]", "mov {t}, qword ptr [{p} + 16]", "mov {t}, qword ptr [{p} + 24]", "dec {n}", "jnz 2b",
            n = inout(reg) iterations => _, p = in(reg) line, t = out(reg) _, options(nostack, readonly)
        );
    }
}

/// `steps` dependent loads along a cycle of pointers
fn chase(start: *const u64, steps: u64) {
    unsafe {
        asm!("2:", "mov {p}, qword ptr [{p}]", "dec {n}", "jnz 2b", p = inout(reg) start => _, n = inout(reg) steps => _, options(nostack, readonly));
    }
}

/// A buffer of `bytes` whose cache lines point at each other in one random cycle (Sattolo's shuffle)
fn cycle(bytes: usize) -> Vec<u64> {
    let lines = (bytes / 64).max(2);
    let mut buffer = vec![0u64; lines * 8];
    let mut order: Vec<usize> = (0..lines).collect();
    let mut random = 0x2545_F491_4F6C_DD1Du64;
    for index in (1..lines).rev() {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        order.swap(index, (random % index as u64) as usize);
    }
    let base = buffer.as_mut_ptr();
    for position in 0..lines {
        let (from, to) = (order[position], order[(position + 1) % lines]);
        unsafe { *base.add(from * 8) = base.add(to * 8) as u64 };
    }
    buffer
}

struct Kernel<'a> {
    name: String,
    /// What the per-unit figures count (an iteration, a load, a step)
    unit: &'static str,
    units: u64,
    run: Box<dyn Fn() + 'a>,
    /// (counter, lowest, highest per unit)
    checks: Vec<(&'static str, f64, f64)>,
}

/// Counts `kernels` with `counters`: each kernel's counts
fn measure(endpoint: client::Endpoint, counters: &[&str], kernels: &[Kernel]) -> Result<Vec<readings::Counts>, String> {
    let reader = client::Reader::new()?;
    let mut service = client::Client::connect(endpoint)?;
    let sources: Vec<String> = counters.iter().map(|name| protocol::source_of(name).to_string()).collect();
    let started = service.start(&sources, &[client::current_thread_id()])?;
    for note in &started.notes {
        println!("  note: {note}");
    }
    let mut at = Vec::with_capacity(kernels.len() * 2);
    for kernel in kernels {
        // (once to warm the caches and the branch predictor up to the kernel, then counted)
        (kernel.run)();
        at.push(reader.read());
        (kernel.run)();
        at.push(reader.read());
    }
    drop(reader);
    let results = service.stop()?;
    for note in &results.notes {
        println!("  note: {note}");
    }
    if results.lost_events > 0 || results.without_counters > 0 {
        println!("  warning: the session lost {} events; {} switches came without counter values", results.lost_events, results.without_counters);
    }
    let thread = results.threads.iter().find(|thread| thread.registered).ok_or("the service sent no slices of the probe's thread")?;
    let frequency = started.qpc_frequency.max(1) as i64;
    let split = readings::split(&at, &thread.slices, frequency / 1000);
    if split.matched < at.len() {
        println!("  warning: {} of {} readings found no switch of their own", at.len() - split.matched, at.len());
    }
    Ok((0..kernels.len()).map(|index| split.windows[2 * index + 1]).collect())
}

/// Runs the kernels and prints what each counter counted: 0 when every check passed, 1 when one failed, 2 when nothing
/// could be counted
pub fn run(endpoint: client::Endpoint) -> i32 {
    let cpu = unsafe { ffi::GetCurrentProcessorNumber() };
    if cpu < usize::BITS {
        unsafe { ffi::SetThreadAffinityMask(ffi::GetCurrentThread(), 1usize << cpu) };
    }
    let (vendor, family) = register::cpu();
    let custom = register::custom_counters_fit();
    println!("ree-hw-counters probe: {vendor} family {family:#x}, pinned to CPU {cpu}; custom cache counters: {}", if custom { "yes" } else { "not defined for this CPU" });
    let line = cycle(64);
    let small = cycle(16 << 10);
    let l2 = cycle(512 << 10);
    let l3 = cycle(8 << 20);
    let memory = cycle(256 << 20);
    let chase_kernel = |name: &str, buffer: &Vec<u64>, steps: u64, checks: Vec<(&'static str, f64, f64)>| {
        let start = buffer.as_ptr() as usize;
        Kernel { name: name.to_string(), unit: "step", units: steps, run: Box::new(move || chase(start as *const u64, steps)), checks }
    };
    let mut first: Vec<&str> = vec!["instructions", "cycles", "branches", "branch-misses"];
    if custom {
        first.extend(["l1d-accesses", "l1d-misses"]);
    }
    let mut loads_checks = vec![];
    if custom {
        loads_checks = vec![("l1d-accesses", 3.5, 5.0), ("l1d-misses", 0.0, 0.01)];
    }
    let line_start = line.as_ptr() as usize;
    let groups: Vec<(Vec<&str>, Vec<Kernel>)> = vec![
        (
            first,
            vec![
                Kernel { name: "spin".into(), unit: "iteration", units: 50_000_000, run: Box::new(|| spin(50_000_000)), checks: vec![("instructions", 3.98, 4.04), ("branches", 0.99, 1.02), ("branch-misses", 0.0, 0.001)] },
                Kernel { name: "random branches".into(), unit: "iteration", units: 20_000_000, run: Box::new(|| branches(20_000_000, 1)), checks: vec![("branches", 1.98, 2.05), ("branch-misses", 0.35, 0.65)] },
                Kernel { name: "steady branches".into(), unit: "iteration", units: 20_000_000, run: Box::new(|| branches(20_000_000, 0)), checks: vec![("branches", 1.98, 2.05), ("branch-misses", 0.0, 0.01)] },
                Kernel { name: "L1 loads".into(), unit: "iteration", units: 25_000_000, run: Box::new(move || loads(25_000_000, line_start as *const u64)), checks: loads_checks },
            ],
        ),
    ];
    let mut groups = groups;
    if custom {
        groups.push((
            vec!["instructions", "cycles", "l1d-misses", "l2-misses", "llc-misses", "branch-misses"],
            vec![
                chase_kernel("chase 16 KB (fits L1)", &small, 4_000_000, vec![("l1d-misses", 0.0, 0.05)]),
                chase_kernel("chase 512 KB (fits L2)", &l2, 4_000_000, vec![("l1d-misses", 0.7, 1.1), ("l2-misses", 0.0, 0.1)]),
                chase_kernel("chase 8 MB (fits L3)", &l3, 2_000_000, vec![("l2-misses", 0.7, 1.1), ("llc-misses", 0.0, 0.15)]),
                chase_kernel("chase 256 MB (memory)", &memory, 2_000_000, vec![("llc-misses", 0.6, 1.1)]),
            ],
        ));
    }
    let (mut checked, mut failed) = (0, 0);
    for (counters, kernels) in &groups {
        println!("\ncounters: {}", counters.join(", "));
        let counts = match measure(endpoint, counters, kernels) {
            Ok(counts) => counts,
            Err(error) => {
                println!("  could not count: {error}");
                return 2;
            }
        };
        for (kernel, counts) in kernels.iter().zip(&counts) {
            let per = |name: &str| counters.iter().position(|counter| *counter == name).map(|index| counts[index] as f64 / kernel.units as f64);
            let ipc = match (per("instructions"), per("cycles")) {
                (Some(instructions), Some(cycles)) if cycles > 0.0 => format!(", IPC {:.2}", instructions / cycles),
                _ => String::new(),
            };
            println!("  {} ({} {}s{ipc})", kernel.name, kernel.units, kernel.unit);
            for (index, counter) in counters.iter().enumerate() {
                let value = counts[index] as f64 / kernel.units as f64;
                let check = kernel.checks.iter().find(|check| check.0 == *counter);
                let verdict = match check {
                    Some(&(_, low, high)) => {
                        checked += 1;
                        if (low..=high).contains(&value) {
                            format!("ok, expected {low}..{high}")
                        } else {
                            failed += 1;
                            format!("FAILED, expected {low}..{high}")
                        }
                    }
                    None => String::new(),
                };
                println!("    {counter:<14} {:>16} total {value:>10.4} a {}  {verdict}", counts[index], kernel.unit);
            }
        }
    }
    println!("\n{} of {checked} checks passed", checked - failed);
    if failed > 0 { 1 } else { 0 }
}
