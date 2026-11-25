// Runs one command and measures its whole process tree: time, CPU, memory peaks, page faults, I/O, processes, handles.
//
// Windows: jobwatch.cs (built on first use with the C# compiler Windows ships) puts the tree in a Job Object and reads
// the job's accounting and every process's own counters; everything is exact except the summed working set, which is
// sampled every 100 ms.
// Linux: the command runs under a shell that waits for it and then copies its own /proc/<pid>/stat and io, which by then
// hold the CPU time, page faults and I/O of every process in the tree that was waited for (the kernel adds a reaped
// child's figures to its parent's), so those totals are exact. Memory peaks (VmHWM, VmRSS), handles (open fds) and the
// process count come from sampling /proc every 100 ms, so a process that lives less than that can be missed.
// macOS: /usr/bin/time -l gives CPU time, the largest process's peak memory and page faults; `ps` sampling gives the
// summed memory and the process count. macOS has no per-process byte counts for I/O and no handle counts: those
// metrics are reported as missing there.
//
// The result's fields are the metric keys of metrics.mjs (bytes, ms, counts), plus `missing` (metrics this platform
// cannot measure), `exitCode` and `perProcess` (Windows).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const INTERVAL_MS = 100;

// The jobwatch.exe for this source, built into `toolsDir` (outside any measured folder) when missing
export function jobwatch(toolsDir) {
	const source = path.join(HERE, "jobwatch.cs");
	const digest = createHash("sha256").update(fs.readFileSync(source)).digest("hex").slice(0, 12);
	const exe = path.join(toolsDir, `jobwatch-${digest}.exe`);
	if (fs.existsSync(exe)) return exe;
	fs.mkdirSync(toolsDir, { recursive: true });
	const windir = process.env.WINDIR || process.env.SystemRoot || "C:\\Windows";
	const csc = [path.join(windir, "Microsoft.NET", "Framework64", "v4.0.30319", "csc.exe"), path.join(windir, "Microsoft.NET", "Framework", "v4.0.30319", "csc.exe")].find(file => fs.existsSync(file));
	if (!csc) throw new Error("tools/resources: the .NET Framework 4 C# compiler (csc.exe) is missing, so the Job Object watcher cannot be built");
	const partial = `${exe}.${process.pid}.tmp`;
	const built = spawnSync(csc, ["-nologo", "-optimize+", "-platform:anycpu", `-out:${partial}`, source], { encoding: "utf8", windowsHide: true });
	if (built.status !== 0) throw new Error(`tools/resources: building jobwatch failed:\n${built.stdout}${built.stderr}`);
	try {
		fs.renameSync(partial, exe);
	} catch (error) {
		fs.rmSync(partial, { force: true });
		if (!fs.existsSync(exe)) throw error;
	}
	return exe;
}

// The CPUs a run of `count` CPUs is held to: the highest `count` of the machine's (the low ones are where a desktop's own
// programs usually run), as a mask; 0 when `count` is 0 or not below what there is
export function cpuMask(count) {
	const cpus = os.cpus().length;
	if (!count || count >= os.availableParallelism() || cpus > 64) return 0n;
	let mask = 0n;
	for (let cpu = cpus - count; cpu < cpus; cpu++) mask |= 1n << BigInt(cpu);
	return mask;
}

// Measures `argv` run in `cwd` with `env`. Its stdout and stderr go to `log`. `onSample` is called about once a second
// while it runs (the folder watcher's sampling). `cpus` holds the tree to that many CPUs (Windows: the job's
// affinity; Linux: taskset when it is installed; macOS has no way to).
export async function measure({ argv, cwd, env, log, toolsDir, lingerMs = 5000, onSample, cpus = 0 }) {
	fs.mkdirSync(path.dirname(log), { recursive: true });
	const ticker = onSample ? setInterval(onSample, 1000) : null;
	try {
		if (process.platform === "win32") return await measureWindows({ argv, cwd, env, log, toolsDir, lingerMs, mask: cpuMask(cpus) });
		if (process.platform === "linux") {
			const mask = cpuMask(cpus);
			const taskset = mask && spawnSync("taskset", ["--version"]).status === 0;
			return await measureLinux({ argv: taskset ? ["taskset", mask.toString(16), ...argv] : argv, cwd, env, log });
		}
		if (process.platform === "darwin") return await measureMac({ argv, cwd, env, log });
		throw new Error(`tools/resources: no process measurement for ${process.platform}`);
	} finally {
		if (ticker) clearInterval(ticker);
	}
}

function waitFor(child) {
	return new Promise((resolve, reject) => {
		child.on("error", reject);
		child.on("exit", (code, signal) => resolve({ code, signal }));
	});
}

async function measureWindows({ argv, cwd, env, log, toolsDir, lingerMs, mask }) {
	const exe = jobwatch(toolsDir);
	const stats = path.join(toolsDir, `stats-${process.pid}-${Date.now()}.json`);
	const child = spawn(exe, ["--out", stats, "--cwd", cwd, "--stdout", log, "--stderr", log, "--interval-ms", String(INTERVAL_MS), "--linger-ms", String(lingerMs), ...(mask ? ["--affinity", mask.toString()] : []), "--", ...argv], { env, stdio: ["ignore", "inherit", "inherit"], windowsHide: true });
	const { code } = await waitFor(child);
	if (!fs.existsSync(stats)) throw new Error(`tools/resources: jobwatch ended (exit ${code}) without its figures`);
	const raw = JSON.parse(fs.readFileSync(stats, "utf8"));
	fs.rmSync(stats, { force: true });
	return {
		platform: "win32",
		exitCode: raw.exitCode,
		wallMs: raw.wallMs,
		cpuMs: raw.cpuUserMs + raw.cpuKernelMs,
		cpuKernelMs: raw.cpuKernelMs,
		pageFaults: raw.pageFaults,
		processes: raw.processes,
		peakProcesses: raw.peakProcesses,
		peakWorkingSetMax: raw.peakWorkingSetMax,
		peakWorkingSetSum: raw.peakWorkingSetSum,
		peakPrivateSum: raw.peakPrivateSum,
		peakPrivateMax: raw.peakPrivateMax,
		peakHandlesMax: raw.peakHandlesMax,
		ioReadBytes: raw.ioReadBytes,
		ioWriteBytes: raw.ioWriteBytes,
		leftRunning: raw.leftRunning.length,
		leftRunningNames: raw.leftRunning,
		missing: [],
		perProcess: summarizeProcesses(raw.perProcess),
	};
}

// Per program name: how many ran and their largest figures
function summarizeProcesses(list) {
	const byImage = new Map();
	for (const proc of list) {
		const name = proc.image || "?";
		const entry = byImage.get(name) || { image: name, count: 0, peakWorkingSet: 0, peakPrivate: 0, writeBytes: 0, readBytes: 0, cpuMs: 0, pageFaults: 0, peakHandles: 0 };
		entry.count++;
		entry.peakWorkingSet = Math.max(entry.peakWorkingSet, proc.peakWorkingSet);
		entry.peakPrivate = Math.max(entry.peakPrivate, proc.peakPrivate);
		entry.peakHandles = Math.max(entry.peakHandles, proc.peakHandles);
		entry.writeBytes += proc.writeBytes;
		entry.readBytes += proc.readBytes;
		entry.cpuMs += proc.cpuUserMs + proc.cpuKernelMs;
		entry.pageFaults += proc.pageFaults;
		byImage.set(name, entry);
	}
	return [...byImage.values()].sort((a, b) => b.peakWorkingSet - a.peakWorkingSet);
}

// /proc/<pid>/stat fields after the command name (which may hold spaces and parentheses)
function statFields(text) {
	return text.slice(text.lastIndexOf(")") + 2).trim().split(/\s+/);
}

function readProc(pid, name) {
	try {
		return fs.readFileSync(`/proc/${pid}/${name}`, "utf8");
	} catch {
		return null;
	}
}

function ioField(text, name) {
	const match = text && text.match(new RegExp(`^${name}:\\s*(\\d+)`, "m"));
	return match ? Number(match[1]) : 0;
}

async function measureLinux({ argv, cwd, env, log }) {
	const ticks = Number(spawnSync("getconf", ["CLK_TCK"], { encoding: "utf8" }).stdout.trim()) || 100;
	const pageSize = Number(spawnSync("getconf", ["PAGESIZE"], { encoding: "utf8" }).stdout.trim()) || 4096;
	const totals = path.join(os.tmpdir(), `resources-${process.pid}-${Date.now()}.txt`);
	const out = fs.openSync(log, "w");
	const started = performance.now();
	// The shell's own stat and io, read once its command has been waited for: they then include the whole waited tree
	const script = '"$@"; status=$?; { cat /proc/$$/stat; echo; cat /proc/$$/io; } > "$RESOURCES_TOTALS" 2>/dev/null; exit $status';
	const child = spawn("/bin/sh", ["-c", script, "resources", ...argv], { cwd, env: { ...env, RESOURCES_TOTALS: totals }, stdio: ["ignore", out, out] });
	const seen = new Map();
	let peakSum = 0, peakFds = 0, peakLive = 0, peakSumFds = 0;
	const sample = () => {
		const tree = descendants(child.pid);
		let sum = 0, sumFds = 0;
		for (const pid of tree) {
			const status = readProc(pid, "status");
			if (!status) continue;
			const hwm = Number((status.match(/^VmHWM:\s*(\d+)/m) || [0, 0])[1]) * 1024;
			const rss = Number((status.match(/^VmRSS:\s*(\d+)/m) || [0, 0])[1]) * 1024;
			let fds = 0;
			try {
				fds = fs.readdirSync(`/proc/${pid}/fd`).length;
			} catch {
				// gone, or not ours
			}
			const entry = seen.get(pid) || { pid, peak: 0, fds: 0, start: statFields(readProc(pid, "stat") || ")  ")[19] };
			entry.peak = Math.max(entry.peak, hwm);
			entry.fds = Math.max(entry.fds, fds);
			seen.set(pid, entry);
			sum += rss;
			sumFds += fds;
		}
		peakSum = Math.max(peakSum, sum);
		peakSumFds = Math.max(peakSumFds, sumFds);
		peakLive = Math.max(peakLive, tree.length);
	};
	const timer = setInterval(sample, INTERVAL_MS);
	const { code, signal } = await waitFor(child);
	clearInterval(timer);
	fs.closeSync(out);
	const wallMs = performance.now() - started;
	const text = fs.existsSync(totals) ? fs.readFileSync(totals, "utf8") : "";
	fs.rmSync(totals, { force: true });
	const fields = statFields(text.split("\n")[0] || "");
	// stat fields (from the state field, 0-based): 7 minflt 8 cminflt 9 majflt 10 cmajflt 11 utime 12 stime 13 cutime 14 cstime
	const num = index => Number(fields[index] || 0);
	const cpuUser = (num(11) + num(13)) / ticks * 1000, cpuKernel = (num(12) + num(14)) / ticks * 1000;
	for (const entry of seen.values()) peakFds = Math.max(peakFds, entry.fds);
	return {
		platform: "linux",
		exitCode: code ?? (signal ? 128 : 1),
		wallMs,
		cpuMs: cpuUser + cpuKernel,
		cpuKernelMs: cpuKernel,
		pageFaults: num(7) + num(8) + num(9) + num(10),
		processes: seen.size,
		peakProcesses: peakLive,
		peakWorkingSetMax: Math.max(0, ...[...seen.values()].map(entry => entry.peak)),
		peakWorkingSetSum: peakSum,
		peakPrivateSum: null,
		peakHandlesMax: peakFds,
		// wchar/rchar: every byte passed to write()/read(), the same thing Windows' I/O accounting counts
		ioReadBytes: ioField(text, "rchar"),
		ioWriteBytes: ioField(text, "wchar"),
		ioStorageWriteBytes: ioField(text, "write_bytes"),
		leftRunning: endLeftovers(seen),
		missing: ["peakPrivateSum"],
		pageSize,
	};
}

// Processes of the tree still alive after its command ended (the same pid with the same start time), ended now as a job
// object would end them on Windows
function endLeftovers(seen) {
	let left = 0;
	for (const entry of seen.values()) {
		const stat = readProc(entry.pid, "stat");
		if (!stat || statFields(stat)[19] !== entry.start) continue;
		left++;
		try {
			process.kill(entry.pid, "SIGKILL");
		} catch {
			// already gone
		}
	}
	return left;
}

// The processes below `root` now (from each process's parent id)
function descendants(root) {
	if (process.platform === "linux") {
		const parents = new Map();
		for (const name of fs.readdirSync("/proc")) {
			if (!/^\d+$/.test(name)) continue;
			const stat = readProc(name, "stat");
			if (!stat) continue;
			parents.set(Number(name), Number(statFields(stat)[1]));
		}
		return collect(root, parents);
	}
	const listed = spawnSync("ps", ["-A", "-o", "pid=,ppid="], { encoding: "utf8" }).stdout || "";
	const parents = new Map(listed.trim().split("\n").map(line => line.trim().split(/\s+/).map(Number)).filter(pair => pair.length === 2).map(([pid, ppid]) => [pid, ppid]));
	return collect(root, parents);
}

function collect(root, parents) {
	const children = new Map();
	for (const [pid, ppid] of parents) {
		if (!children.has(ppid)) children.set(ppid, []);
		children.get(ppid).push(pid);
	}
	const out = [];
	const stack = [...(children.get(root) || [])];
	while (stack.length) {
		const pid = stack.pop();
		out.push(pid);
		stack.push(...(children.get(pid) || []));
	}
	return out;
}

async function measureMac({ argv, cwd, env, log }) {
	const totals = path.join(os.tmpdir(), `resources-${process.pid}-${Date.now()}.txt`);
	const out = fs.openSync(log, "w");
	const started = performance.now();
	const child = spawn("/usr/bin/time", ["-l", "-o", totals, ...argv], { cwd, env, stdio: ["ignore", out, out] });
	const seen = new Set();
	let peakSum = 0, peakLive = 0;
	const timer = setInterval(() => {
		const tree = descendants(child.pid);
		if (!tree.length) return;
		const listed = spawnSync("ps", ["-o", "pid=,rss=", "-p", tree.join(",")], { encoding: "utf8" }).stdout || "";
		let sum = 0;
		for (const line of listed.trim().split("\n")) {
			const [pid, rss] = line.trim().split(/\s+/).map(Number);
			if (!pid) continue;
			seen.add(pid);
			sum += rss * 1024;
		}
		peakSum = Math.max(peakSum, sum);
		peakLive = Math.max(peakLive, tree.length);
	}, INTERVAL_MS);
	const { code, signal } = await waitFor(child);
	clearInterval(timer);
	fs.closeSync(out);
	const wallMs = performance.now() - started;
	const text = fs.existsSync(totals) ? fs.readFileSync(totals, "utf8") : "";
	fs.rmSync(totals, { force: true });
	const value = label => Number((text.match(new RegExp(`(\\d+)\\s+${label}`)) || [0, 0])[1]);
	const seconds = label => Number((text.match(new RegExp(`([\\d.]+)\\s+${label}`)) || [0, 0])[1]) * 1000;
	return {
		platform: "darwin",
		exitCode: code ?? (signal ? 128 : 1),
		wallMs,
		cpuMs: seconds("user") + seconds("sys"),
		cpuKernelMs: seconds("sys"),
		pageFaults: value("page reclaims") + value("page faults"),
		processes: seen.size + 1,
		peakProcesses: peakLive,
		peakWorkingSetMax: value("maximum resident set size"),
		peakWorkingSetSum: peakSum,
		peakPrivateSum: null,
		peakHandlesMax: null,
		ioReadBytes: null,
		ioWriteBytes: null,
		blockReads: value("block input operations"),
		blockWrites: value("block output operations"),
		leftRunning: [...seen].filter(pid => spawnSync("ps", ["-p", String(pid)], { encoding: "utf8" }).status === 0).length,
		missing: ["peakPrivateSum", "peakHandlesMax", "ioReadBytes", "ioWriteBytes"],
	};
}
