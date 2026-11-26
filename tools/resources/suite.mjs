// ree's resource workloads (tools/resources/measure.mjs runs them; AGENTS.md "Resource budgets" says when).
//
// The batch, stress and service workloads run a project this file generates into the run's folder: a library of
// modules every job requires (its compiled bytecode fills the module store, as a real game project's does) and a few
// modules of each job's own (so every job publishes a new pack to the store, and the store compacts every 16 packs).
// The sizes are chosen so the store matters to the figures: a store that rewrote itself on every flush multiplies the
// data written, and processes that read each new compacted pack whole raise the summed memory and the page faults well
// past their budgets. The recording runs a scene of its own (fixtures/Scene.luau), the parity probes what the repository
// already has (tools/parity/probes: 23 of them in CI, chosen by name, all of them in the full suite).
//
// Options: --ree <exe> (default: target/release/ree beside this checkout, or REE_BIN).

import fs from "node:fs";
import path from "node:path";

export const FLAGS = [];

export const SUITES = {
	ci: { about: "What CI runs on every push and pull request: about a minute on a 4-CPU runner.", workloads: ["batch-cold", "batch-warm", "store-stress", "serve-series", "record", "parity-sample"] },
	full: { about: "The CI set at a larger size, for checking a store or worker change locally.", workloads: ["batch-cold-full", "batch-warm-full", "store-stress-full", "serve-series", "record", "parity-probes"] },
};

const EXE = process.platform === "win32" ? "ree.exe" : "ree";

function reeBinary(context) {
	const given = context.args.ree || process.env.REE_BIN;
	const binary = path.resolve(given || path.join(context.repo, "target", "release", EXE));
	if (!fs.existsSync(binary)) throw new Error(`tools/resources: no ree at ${binary} (cargo build --release, or pass --ree <exe>)`);
	return binary;
}

// A deterministic module of about `bytes` characters: functions with their own constants and strings, a data table,
// and the name it is required by, so every module compiles to its own bytecode
function moduleSource(name, seed, bytes) {
	const lines = ["local M = {}", `M.Name = "${name}"`];
	let state = seed * 2654435761 % 4294967296;
	const next = () => (state = (state * 1103515245 + 12345) % 2147483648);
	let size = 40, index = 0;
	while (size < bytes) {
		const a = next() % 97 + 3, b = next() % 89 + 2, c = next() % 1000 / 7;
		const body = [
			`function M.F${index}(X: number, Y: number): number`,
			"\tlocal Total = 0",
			`\tfor I = 1, X do Total += (I * ${a} + Y) % ${b} + ${c.toFixed(3)} end`,
			`\tif Total > ${next() % 5000} then return Total - ${a} else return Total * ${b % 7 + 1} end`,
			"end",
			`M.K${index} = { Key = "${name}.${index}.${next().toString(36)}", Weight = ${(next() % 100000) / 100}, Tags = { "${next().toString(36)}", "${next().toString(36)}" } }`,
		];
		for (const line of body) size += line.length + 1;
		lines.push(...body);
		index++;
	}
	lines.push(`M.Count = ${index}`, "return M", "");
	return lines.join("\n");
}

// The generated project (written once per run folder; not measured): shared modules, job modules, the harness
function project(context, size) {
	const root = path.join(context.work, `project-${size.name}`);
	if (fs.existsSync(path.join(root, "done"))) return root;
	const shared = path.join(root, "lib", "Shared");
	fs.mkdirSync(shared, { recursive: true });
	for (let index = 0; index < size.shared; index++) {
		const name = `S${String(index).padStart(4, "0")}`;
		fs.writeFileSync(path.join(shared, `${name}.luau`), moduleSource(name, index + 1, size.sharedBytes));
	}
	for (let job = 0; job < size.jobs; job++) {
		const folder = path.join(root, "lib", "Jobs", `J${String(job).padStart(4, "0")}`);
		fs.mkdirSync(folder, { recursive: true });
		for (let index = 0; index < size.own; index++) {
			const name = `J${job}U${index}`;
			fs.writeFileSync(path.join(folder, `${name}.luau`), moduleSource(name, 100000 + job * 100 + index, size.ownBytes));
		}
	}
	fs.writeFileSync(path.join(root, "default.project.json"), JSON.stringify({
		name: "resource-suite",
		tree: { $className: "DataModel", ReplicatedStorage: { $className: "ReplicatedStorage", Lib: { $path: "lib" } } },
	}, null, "\t"));
	// Requires every shared module and the job's own, calls one function of each, and ends
	fs.writeFileSync(path.join(root, "Job.luau"), [
		"return function(Context)",
		"\tlocal Lib = game:GetService(\"ReplicatedStorage\"):WaitForChild(\"Lib\")",
		"\tlocal Total = 0",
		"\tfor _, Module in Lib.Shared:GetChildren() do",
		"\t\tTotal += require(Module).F0(3, Context.Job)",
		"\tend",
		"\tfor _, Slice in Context.Slices do",
		"\t\tlocal Folder = Lib.Jobs:FindFirstChild(string.format(\"J%04d\", Slice))",
		"\t\tfor _, Module in (if Folder then Folder:GetChildren() else {}) do",
		"\t\t\tTotal += require(Module).F0(2, 1)",
		"\t\tend",
		"\tend",
		"\tprint(\"total\", Total)",
		"\tprint(\"@done\")",
		"end",
		"",
	].join("\n"));
	fs.writeFileSync(path.join(root, "done"), "");
	return root;
}

// Job `job` requires the core, its own slice, the slice before it (another process compiles that one, so it usually comes
// from a pack) and `size.wide + 1` older slices (by then in the newest compacted pack): so each process keeps loading
// from the newest packs, as a long-lived worker over a real project does, and holds them mapped while it keeps their
// bytecode
function jobArgs(root, job, size) {
	const older = Array.from({ length: Math.min((size.wide || 0) + 1, job) }, (_, index) => (job * 37 + index * 13) % job);
	const slices = [...new Set([job, ...(job > 0 ? [job - 1] : []), ...older])];
	return ["--harness", path.join(root, "Job.luau"), "--rojo", path.join(root, "default.project.json"), "--seconds", "1", "--context", JSON.stringify({ Job: job, Slices: slices })];
}

// The folders a ree workload uses: its state (module store and the other caches), its output and its temp folder
function folders(context, id, state) {
	return [
		{ label: "State folder (module store, caches)", dir: state, kind: "state" },
		{ label: "Output folder", dir: path.join(context.work, id, "out"), kind: "out" },
		{ label: "Temp folder", dir: path.join(context.work, id, "temp"), kind: "temp" },
	];
}

// `processes` `ree run --batch` processes side by side on one state folder, each running its share of the jobs on
// `workers` threads
function batchWorkload({ id, title, about, size, processes, workers, stateOf, fresh, requires }) {
	return {
		id, title, about, requires,
		prepare(context) {
			const ree = reeBinary(context);
			const root = project(context, size);
			const state = path.join(context.work, stateOf);
			if (fresh) fs.rmSync(state, { recursive: true, force: true });
			fs.mkdirSync(state, { recursive: true });
			const out = path.join(context.work, id, "out");
			fs.rmSync(path.join(context.work, id), { recursive: true, force: true });
			fs.mkdirSync(out, { recursive: true });
			const parallel = [];
			for (let index = 0; index < processes; index++) {
				const jobs = [];
				for (let job = index; job < size.jobs; job += processes) jobs.push({ id: `job${job}`, args: jobArgs(root, job, size) });
				const spec = path.join(out, `batch-${index}.json`);
				fs.writeFileSync(spec, JSON.stringify({ workers, jobs }));
				parallel.push({ argv: [ree, "run", "--batch", spec], log: path.join(out, `batch-${index}.log`) });
			}
			const plan = path.join(out, "plan.json");
			fs.writeFileSync(plan, JSON.stringify({ steps: [{ parallel }] }));
			return { argv: [context.node, context.fanout, plan], folders: folders(context, id, state), env: { RUR_STATE_DIR: state }, out, jobs: size.jobs, processes };
		},
		// every job ran and passed (a batch prints {"results": [...]} on stdout, into its log)
		verify(prepared) {
			let passed = 0;
			for (let index = 0; index < prepared.processes; index++) {
				const log = fs.readFileSync(path.join(prepared.out, `batch-${index}.log`), "utf8");
				const line = log.split("\n").find(text => text.startsWith("{\"results\""));
				if (!line) return `batch ${index} printed no results`;
				passed += JSON.parse(line).results.filter(result => result.code === 0).length;
			}
			return passed === prepared.jobs ? null : `${passed} of ${prepared.jobs} jobs passed`;
		},
	};
}

// The sample of `size` probes: the ones whose names hash lowest (FNV-1a), in name order. A hash, not a place in the list,
// so a probe added or removed moves at most itself in and one other out (by place, three new probes once changed 20 of
// the 25 sampled and the figures with them); a fixed size, not a fraction of the hashes, so the sample does not grow
// with the probes (one in five by hash, a day of new probes took it from 23 to 30, and its processes, two a probe, over
// their budget)
export function sampled(names, size) {
	const hash = name => {
		let value = 0x811c9dc5;
		for (const byte of Buffer.from(name, "utf8")) value = Math.imul(value ^ byte, 0x01000193) >>> 0;
		return value;
	};
	const ranked = names.map(name => ({ name, hash: hash(name) })).sort((a, b) => a.hash - b.hash || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
	return ranked.slice(0, size).map(entry => entry.name).sort();
}

// `node tools/parity/run.mjs` over the `sample` probes `sampled` picks (all of them when `sample` is not given)
function parityWorkload({ id, title, about, sample }) {
	return {
		id, title, about,
		prepare(context) {
			const ree = reeBinary(context);
			const state = path.join(context.work, `${id}-state`);
			fs.rmSync(state, { recursive: true, force: true });
			fs.mkdirSync(state, { recursive: true });
			const probes = fs.readdirSync(path.join(context.repo, "tools", "parity", "probes")).filter(name => name.endsWith(".luau")).map(name => name.slice(0, -5)).sort();
			const chosen = sample ? sampled(probes, sample) : [];
			return { argv: [context.node, path.join(context.repo, "tools", "parity", "run.mjs"), ...chosen, "--ree", ree], cwd: context.repo, folders: folders(context, id, state), env: { RUR_STATE_DIR: state } };
		},
	};
}

// The generated projects: `shared` core modules every job requires, `jobs` slices of `own` modules, each slice a job's own;
// `wide`: how many more older slices a job loads from the store
const CI_SIZE = { name: "ci", shared: 24, sharedBytes: 8000, jobs: 144, own: 8, ownBytes: 32000 };
const STRESS_SIZE = { name: "stress", shared: 24, sharedBytes: 8000, jobs: 360, own: 4, ownBytes: 28000, wide: 12 };
const FULL_SIZE = { name: "full", shared: 48, sharedBytes: 8000, jobs: 384, own: 6, ownBytes: 40000 };
const FULL_STRESS_SIZE = { name: "full-stress", shared: 48, sharedBytes: 8000, jobs: 720, own: 4, ownBytes: 28000, wide: 16 };

export function workloads() {
	return [
		batchWorkload({
			id: "batch-cold", title: "Batch, cold module store",
			about: "144 jobs in 3 `ree run --batch` processes of 2 workers each, on a new state folder: every module is compiled and saved to the module store (about 37 MB), which compacts every 16 new packs.",
			size: CI_SIZE, processes: 3, workers: 2, stateOf: "batch-state", fresh: true,
		}),
		batchWorkload({
			id: "batch-warm", title: "Batch, warm module store",
			about: "The same 144 jobs again on the state folder the cold run left: every module comes from the store, so almost nothing should be written.",
			size: CI_SIZE, processes: 3, workers: 2, stateOf: "batch-state", fresh: false, requires: ["batch-cold"],
		}),
		batchWorkload({
			id: "store-stress", title: "Many processes sharing one cold store",
			about: "360 jobs over 12 `ree run --batch` processes of one worker each, all starting together on one new state folder, each job also loading 14 older slices from the store: where compactions pile up and every process maps every new pack, as many render workers do.",
			size: STRESS_SIZE, processes: 12, workers: 1, stateOf: "stress-state", fresh: true,
		}),
		batchWorkload({
			id: "batch-cold-full", title: "Batch, cold module store (full size)",
			about: "384 jobs in 4 processes of 3 workers on a new state folder, a module store of about 90 MB.",
			size: FULL_SIZE, processes: 4, workers: 3, stateOf: "full-state", fresh: true,
		}),
		batchWorkload({
			id: "batch-warm-full", title: "Batch, warm module store (full size)",
			about: "The same 384 jobs on the state folder the full cold run left.",
			size: FULL_SIZE, processes: 4, workers: 3, stateOf: "full-state", fresh: false, requires: ["batch-cold-full"],
		}),
		batchWorkload({
			id: "store-stress-full", title: "Many processes sharing one cold store (full size)",
			about: "720 jobs over 24 processes of one worker each on one new state folder, each job loading 18 older slices.",
			size: FULL_STRESS_SIZE, processes: 24, workers: 1, stateOf: "full-stress-state", fresh: true,
		}),
		{
			id: "serve-series", title: "Warm service, a series of jobs",
			about: "12 `ree run --service` jobs one after another (the second starts the `ree serve` service, which keeps compiled modules and prepares ahead), then `ree serve --stop`.",
			prepare(context) {
				const ree = reeBinary(context);
				const root = project(context, CI_SIZE);
				const state = path.join(context.work, "serve-state");
				fs.rmSync(state, { recursive: true, force: true });
				fs.mkdirSync(state, { recursive: true });
				const out = path.join(context.work, "serve-series", "out");
				fs.rmSync(path.join(context.work, "serve-series"), { recursive: true, force: true });
				fs.mkdirSync(out, { recursive: true });
				const steps = [];
				for (let job = 0; job < 12; job++) steps.push({ argv: [ree, "run", "--service", ...jobArgs(root, job % 4, CI_SIZE)], log: path.join(out, `job-${job}.log`) });
				steps.push({ argv: [ree, "serve", "--stop"], log: path.join(out, "stop.log") });
				const plan = path.join(out, "plan.json");
				fs.writeFileSync(plan, JSON.stringify({ steps }));
				return { argv: [context.node, context.fanout, plan], folders: folders(context, "serve-series", state), env: { RUR_STATE_DIR: state, REE_SERVE_WORKERS: "2" }, lingerMs: 10000 };
			},
		},
		{
			id: "record", title: "Recording a 3D scene",
			about: "`ree record` of tools/resources/fixtures/Scene.luau (parts that move, spin, appear and go away) for 3 seconds at 60 frames a second: the world, the recorder and its files.",
			prepare(context) {
				const ree = reeBinary(context);
				const state = path.join(context.work, "record-state");
				const out = path.join(context.work, "record", "out");
				const empty = path.join(context.work, "record", "empty");
				fs.rmSync(path.join(context.work, "record"), { recursive: true, force: true });
				fs.rmSync(state, { recursive: true, force: true });
				fs.mkdirSync(empty, { recursive: true });
				fs.mkdirSync(state, { recursive: true });
				const harness = path.join(context.repo, "tools", "resources", "fixtures", "Scene.luau");
				return { argv: [ree, "record", "--harness", harness, "--roots", empty, "--out", out, "--seconds", "3", "--fps", "60"], folders: folders(context, "record", state), env: { RUR_STATE_DIR: state } };
			},
		},
		parityWorkload({
			id: "parity-sample", title: "A sample of the parity probes",
			about: "`node tools/parity/run.mjs` with 23 of the probes in tools/parity/probes (chosen by name, so the sample keeps its size as probes are added, and a new probe moves only itself in and one other out), each in a `ree run` process of its own, compared with what a Roblox client printed.",
			sample: 23,
		}),
		parityWorkload({
			id: "parity-probes", title: "The parity probes",
			about: "`node tools/parity/run.mjs`: every probe in tools/parity/probes, each in a `ree run` process of its own, compared with what a Roblox client printed.",
		}),
	];
}
