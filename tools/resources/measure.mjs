#!/usr/bin/env node
// Resource budgets: runs this repository's heavy workloads, measures what each one costs the machine (disk writes, disk
// space, leftover files, memory, page faults, processes; time and CPU are shown only) and checks the figures against
// tools/resources/baseline.json. The report says in plain words what each figure means and whether it is within budget.
//
//   node tools/resources/measure.mjs [--suite ci] [--workload <id,...>] [--compare baseline|<run.json>|none]
//        [--repeat <n>] [--out <run.json>] [--summary <file.md>] [--history <file.jsonl>] [--work <dir>] [--keep]
//        [--write-baseline [--from <run.json>,...]] [--cpus 4] [--where <text>] [--check <run.json>] [--list]
//        [suite options, below]
//   node tools/resources/measure.mjs --run [--watch <dir>[=state|out|temp]]... -- <command...>
//
//   --suite      which workloads (--list shows them); default ci, the set CI runs
//   --compare    baseline (default): this platform's figures in baseline.json; or a run file written by --out, to
//                compare two builds on one machine (a before and an after)
//   --repeat     runs each workload n times (cold and warm pairs stay paired) and checks the median; use 3 or more
//                before --write-baseline so the spread is known
//   --out        writes the run's figures (every repeat, and the medians) as JSON
//   --summary    appends the Markdown report to a file (CI passes $GITHUB_STEP_SUMMARY)
//   --history    appends one JSON line with the medians (CI keeps these on main; history.mjs prints them as a table)
//   --write-baseline   records this run's medians (or those of the --from run files) as this platform's baseline.
//                A deliberate increase goes into baseline.json in the same commit as the change that causes it.
//   --cpus       holds every workload to this many CPUs (default 4, a CI runner's; 0 for all) and tells thread pools
//                (RAYON_NUM_THREADS) the same, so a run on a large machine reads like a runner's
//   --check      checks the figures of a run file written by --out against the baseline (or --compare), without
//                running anything: the same report and exit code as the run itself would have given
//   --run        measures any one command instead of a suite (the folders given with --watch are accounted too)
//
// Exit code: 1 when a workload went over budget or its command failed, 2 for a usage error, else 0.
// The workloads, their fixtures and options are this repository's own: tools/resources/suite.mjs.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { measure } from "./lib/tree.mjs";
import { watch } from "./lib/dirs.mjs";
import { check, entryFrom, platformKey, readBaseline, tooNoisy, verdict, writeBaseline } from "./lib/budget.mjs";
import { markdown, text } from "./lib/report.mjs";
import * as suite from "./suite.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..");
const BASELINE = path.join(HERE, "baseline.json");
// A service or worker that has just exited can hold a handle in the run's folder for a few ms more on Windows (EBUSY,
// EPERM): its removal is tried again
const REMOVE_WORK = { recursive: true, force: true, maxRetries: 5, retryDelay: 200 };

function parseArgs(argv) {
	const args = { watch: [], rest: null };
	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "--") {
			args.rest = argv.slice(i + 1);
			break;
		}
		if (!arg.startsWith("--")) throw usage(`unexpected argument ${arg}`);
		const [key, inline] = arg.slice(2).split(/=(.*)/s);
		const flag = ["list", "keep", "write-baseline", "run"].includes(key) || suite.FLAGS?.includes(key);
		const value = flag ? true : inline ?? argv[++i];
		if (value === undefined) throw usage(`--${key} needs a value`);
		if (key === "watch") args.watch.push(value);
		else args[key] = value;
	}
	return args;
}

function usage(message) {
	const error = new Error(`${message}\nusage: node tools/resources/measure.mjs [--suite <name>] [--workload <id,...>] [--compare baseline|<run.json>|none] [--repeat <n>] [--out <file>] [--summary <file>] [--history <file>] [--write-baseline] [--list]\n       node tools/resources/measure.mjs --run [--watch <dir>[=kind]]... -- <command...>`);
	error.usage = true;
	return error;
}

function git(...args) {
	const result = spawnSync("git", args, { cwd: REPO, encoding: "utf8", windowsHide: true });
	return result.status === 0 ? result.stdout.trim() : null;
}

function where(args, held) {
	if (args.where) return args.where;
	const cpus = held && held < os.availableParallelism() ? `${held} of ${os.availableParallelism()}` : os.availableParallelism();
	if (process.env.GITHUB_ACTIONS) return `GitHub Actions ${process.env.ImageOS || process.env.RUNNER_OS || ""} runner, ${cpus} CPUs`.replace(/\s+/g, " ");
	return `a local machine, ${cpus} CPUs`;
}

// The environment every workload starts from: this one without the variables the tools read their settings from, so
// a developer's own settings never change what is measured
function baseEnv() {
	const env = {};
	for (const [key, value] of Object.entries(process.env)) {
		if (/^(RUR|REE|ROR)_/i.test(key) || /^CARGO_/i.test(key)) continue;
		env[key] = value;
	}
	return env;
}

function median(values) {
	const sorted = [...values].sort((a, b) => a - b);
	const mid = Math.floor(sorted.length / 2);
	return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

// One workload, once: its preparation (not measured), the measured command, its folders
async function runOnce(workload, context) {
	const prepared = await workload.prepare(context);
	const folders = watch(prepared.folders || []);
	folders.start();
	const env = { ...context.env, ...(prepared.env || {}) };
	const temp = (prepared.folders || []).find(folder => folder.kind === "temp");
	if (temp) {
		fs.mkdirSync(temp.dir, { recursive: true });
		Object.assign(env, { TEMP: temp.dir, TMP: temp.dir, TMPDIR: temp.dir });
	}
	// a workload that only runs on a developer's machine may use more CPUs than a runner has
	const cpus = prepared.cpus ?? context.cpus;
	if (cpus) env.RAYON_NUM_THREADS = String(cpus);
	const log = prepared.log || path.join(context.work, workload.id, "command.log");
	const result = await measure({ argv: prepared.argv, cwd: prepared.cwd || context.work, env, log, toolsDir: context.tools, lingerMs: prepared.lingerMs, onSample: () => folders.sample(), cpus });
	const { totals, perFolder } = folders.finish();
	const metrics = {};
	for (const key of ["ioWriteBytes", "ioReadBytes", "peakWorkingSetMax", "peakWorkingSetSum", "peakPrivateSum", "pageFaults", "processes", "peakProcesses", "peakHandlesMax", "leftRunning", "wallMs", "cpuMs", "cpuKernelMs"]) metrics[key] = result[key];
	Object.assign(metrics, totals);
	delete metrics.filesDeleted;
	// (a workload can accept other exit codes, such as a check that reports differences it does not measure)
	const accepted = (prepared.acceptExit || [0]).includes(result.exitCode);
	const problem = accepted && workload.verify ? workload.verify(prepared, result) : null;
	return {
		exitCode: !accepted ? result.exitCode || 1 : problem ? 1 : 0,
		problem,
		log,
		metrics,
		folders: perFolder,
		leftRunningNames: result.leftRunningNames,
		perProcess: result.perProcess,
		missing: result.missing,
	};
}

function summarize(runs) {
	const metrics = {};
	for (const key of Object.keys(runs[0].metrics)) {
		const values = runs.map(run => run.metrics[key]).filter(value => value != null && !Number.isNaN(value));
		metrics[key] = values.length ? median(values) : null;
	}
	// the run closest to the medians stands for the others in the report's folder and per-program lines
	const typical = runs[Math.floor(runs.length / 2)];
	return { ...typical, exitCode: runs.find(run => run.exitCode !== 0)?.exitCode ?? 0, metrics, runs };
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const platform = platformKey();
	const tools = path.resolve(args.tools || path.join(process.env.RUNNER_TEMP || os.tmpdir(), "resource-budgets-tools"));
	const work = path.resolve(args.work || path.join(process.env.RUNNER_TEMP || os.tmpdir(), "resource-budgets", `${args.suite || "ci"}-${process.pid}`));
	const cpus = args.cpus === undefined ? 4 : Number(args.cpus);
	const context = { repo: REPO, work, tools, args, env: baseEnv(), node: process.execPath, fanout: path.join(HERE, "lib", "fanout.mjs"), platform, cpus };

	if (args.run) {
		if (!args.rest?.length) throw usage("--run needs -- <command...>");
		const folders = args.watch.map((spec, index) => {
			const [dir, kind = "state"] = spec.split(/=(?=state$|out$|temp$)/);
			return { label: `folder ${index + 1}`, dir: path.resolve(dir), kind };
		});
		const workload = { id: "command", title: `Command: ${args.rest.join(" ")}`, about: "One command, measured as it was given.", prepare: () => ({ argv: args.rest, cwd: process.cwd(), folders }) };
		fs.mkdirSync(work, { recursive: true });
		const result = await runOnce(workload, context);
		const rows = check(result, null);
		console.log(text({ suite: "command", platform, against: "nothing", checked: [{ workload, rows, result, verdict: result.exitCode === 0 ? "new" : "fail" }] }));
		if (args.out) fs.writeFileSync(args.out, `${JSON.stringify({ platform, workloads: { command: { ...result } } }, null, "\t")}\n`);
		if (!args.keep) fs.rmSync(work, REMOVE_WORK);
		return result.exitCode === 0 ? 0 : 1;
	}

	const all = suite.workloads(context);
	const byId = new Map(all.map(workload => [workload.id, workload]));
	if (args.list) {
		for (const [name, entry] of Object.entries(suite.SUITES)) console.log(`suite ${name}: ${entry.about}\n   ${entry.workloads.join(", ")}`);
		console.log("");
		for (const workload of all) console.log(`${workload.id}: ${workload.title}\n   ${workload.about}`);
		return 0;
	}
	const saved = args.check ? JSON.parse(fs.readFileSync(args.check, "utf8")) : null;
	const suiteName = args.suite || saved?.suite || "ci";
	const chosen = args.workload ? args.workload.split(",") : saved && !args.suite ? Object.keys(saved.workloads || {}) : suite.SUITES[suiteName]?.workloads;
	if (!chosen) throw usage(`no suite ${suiteName} (${Object.keys(suite.SUITES).join(", ")})`);
	for (const id of chosen) if (!byId.has(id)) throw usage(`no workload ${id} (--list shows them)`);

	if (args["write-baseline"] && args.from) {
		const files = args.from.split(",");
		const runs = files.map(file => JSON.parse(fs.readFileSync(file, "utf8")));
		const entries = {};
		for (const id of chosen) {
			const results = runs.flatMap(run => run.workloads?.[id]?.runs || []);
			if (!results.length) continue;
			const first = runs.find(run => run.workloads?.[id]);
			entries[id] = entryFrom(results, { commit: first.commit, date: first.date, where: first.where });
		}
		writeBaseline(BASELINE, runs[0].platform || platform, entries);
		console.log(`baseline.json: ${Object.keys(entries).length} workloads recorded for ${runs[0].platform || platform} from ${files.length} run files`);
		return 0;
	}

	const repeat = saved ? 0 : Math.max(1, Number(args.repeat) || 1);
	if (!saved) {
		fs.rmSync(work, REMOVE_WORK);
		fs.mkdirSync(work, { recursive: true });
	}
	const order = [];
	const add = id => {
		if (order.includes(id)) return;
		for (const need of byId.get(id).requires || []) add(need);
		order.push(id);
	};
	chosen.forEach(add);
	const results = new Map();
	for (const id of saved ? chosen : []) if (saved.workloads[id]) results.set(id, saved.workloads[id].runs);
	for (let round = 0; round < repeat; round++) {
		for (const id of order) {
			const workload = byId.get(id);
			const skip = workload.skip?.(context);
			if (skip) {
				if (round === 0) console.error(`${id}: skipped (${skip})`);
				continue;
			}
			const measured = chosen.includes(id);
			process.stderr.write(`${measured ? "measuring" : "preparing"} ${id}${repeat > 1 ? ` (round ${round + 1} of ${repeat})` : ""}... `);
			const result = await runOnce(workload, context);
			process.stderr.write(`${result.exitCode === 0 ? "done" : `failed (exit ${result.exitCode}${result.problem ? `: ${result.problem}` : ""}, log ${result.log})`} in ${(result.metrics.wallMs / 1000).toFixed(1)} s\n`);
			if (measured) {
				if (!results.has(id)) results.set(id, []);
				results.get(id).push(result);
			}
		}
	}

	const commit = saved ? saved.commit : process.env.GITHUB_SHA || git("rev-parse", "HEAD");
	const date = saved ? saved.date : new Date().toISOString();
	const run = { suite: suiteName, platform: saved?.platform || platform, commit, date, where: saved?.where || where(args, cpus), cpus: saved?.cpus || cpus || os.availableParallelism(), workloads: {} };
	for (const [id, runs] of results) run.workloads[id] = { metrics: summarize(runs).metrics, runs: runs.map(({ metrics, exitCode, folders, perProcess, leftRunningNames }) => ({ metrics, exitCode, folders, perProcess, leftRunningNames })) };

	const compare = args.compare || "baseline";
	let against = "nothing (--compare none)";
	let bases = {};
	const overrides = readBaseline(BASELINE).tolerances || {};
	if (compare === "baseline") {
		bases = readBaseline(BASELINE).platforms?.[platform] || {};
		against = `the ${platform} baseline in tools/resources/baseline.json`;
	} else if (compare !== "none") {
		const other = JSON.parse(fs.readFileSync(compare, "utf8"));
		for (const [id, entry] of Object.entries(other.workloads || {})) bases[id] = { metrics: entry.metrics };
		against = `${path.basename(compare)} (${other.commit ? other.commit.slice(0, 7) : "?"}, ${other.where || "?"})`;
	}
	const checked = [];
	for (const [id, runs] of results) {
		const result = summarize(runs);
		const rows = check(result, bases[id], overrides);
		checked.push({ workload: byId.get(id), rows, result, verdict: verdict(rows, result) });
	}
	const report = { suite: suiteName, platform, against, checked };
	console.log(text(report));
	if (args.summary) fs.appendFileSync(args.summary, markdown(report));
	if (args.out) fs.writeFileSync(args.out, `${JSON.stringify(run, null, "\t")}\n`);
	if (args.history) fs.appendFileSync(args.history, `${JSON.stringify({ suite: suiteName, platform, commit, date, where: run.where, workloads: Object.fromEntries(Object.entries(run.workloads).map(([id, entry]) => [id, entry.metrics])) })}\n`);

	if (args["write-baseline"]) {
		const entries = {};
		for (const [id, runs] of results) {
			if (runs.some(result => result.exitCode !== 0)) {
				console.error(`${id}: not recorded, its command failed`);
				continue;
			}
			entries[id] = entryFrom(runs, { commit, date: date.slice(0, 10), where: run.where });
			for (const noisy of tooNoisy(entries[id], overrides)) console.error(`${id}: ${noisy.key} varied ${noisy.spread}x between runs, more than its budget allows (${noisy.ratio}x): it may flake`);
		}
		writeBaseline(BASELINE, platform, entries);
		console.log(`baseline.json: ${Object.keys(entries).length} workloads recorded for ${platform} (${repeat} runs each)`);
	}
	if (!args.keep && !saved) fs.rmSync(work, REMOVE_WORK);
	else if (!saved) console.error(`kept the run's folders in ${work}`);
	if (args["write-baseline"]) return checked.some(item => item.result.exitCode !== 0) ? 1 : 0;
	return checked.some(item => item.verdict === "fail") ? 1 : 0;
}

main().then(code => process.exit(code), error => {
	console.error(error.usage ? error.message : error.stack || error.message);
	process.exit(error.usage ? 2 : 1);
});
