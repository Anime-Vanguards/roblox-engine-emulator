#!/usr/bin/env node
// Prints the resource suite's history as a table per workload, oldest run first, so a trend shows at a glance.
//
//   node tools/resources/history.mjs <history.jsonl | run.json | folder>... [--workload <id,...>] [--metric <key,...>]
//   node tools/resources/history.mjs --github <owner/repo> [--branch main] [--limit 30] [--workload ...] [--metric ...]
//
// The files are what measure.mjs writes: --history appends a JSON line per run, --out a run file. A folder is searched
// for both. --github downloads the "resource-figures" artifacts CI keeps for each run on main (with the gh CLI,
// signed in) and reads their history lines. --metric picks the columns (metric keys, metrics.mjs); the default is the
// gated figures.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { METRIC, METRICS, format } from "./lib/metrics.mjs";

function parseArgs(argv) {
	const args = { files: [] };
	for (let i = 0; i < argv.length; i++) {
		if (argv[i].startsWith("--")) args[argv[i].slice(2)] = argv[++i];
		else args.files.push(argv[i]);
	}
	return args;
}

// Every run in `file`: a history line has `workloads: { id: metrics }`, a run file `workloads: { id: { metrics } }`
function readRuns(file) {
	const stat = fs.statSync(file);
	if (stat.isDirectory()) return fs.readdirSync(file).flatMap(name => /\.(jsonl|json)$/.test(name) ? readRuns(path.join(file, name)) : []);
	const text = fs.readFileSync(file, "utf8");
	const records = file.endsWith(".jsonl") ? text.split("\n").filter(Boolean).map(line => JSON.parse(line)) : [JSON.parse(text)];
	return records.filter(record => record.workloads).map(record => ({
		date: record.date, commit: record.commit, platform: record.platform, where: record.where,
		workloads: Object.fromEntries(Object.entries(record.workloads).map(([id, entry]) => [id, entry.metrics || entry])),
	}));
}

function fromGitHub(repo, limit, branch) {
	const gh = (...args) => {
		const result = spawnSync("gh", args, { encoding: "utf8", windowsHide: true, maxBuffer: 64 << 20 });
		if (result.status !== 0) throw new Error(`gh ${args.join(" ")}: ${result.stderr.trim()}`);
		return result.stdout;
	};
	const listed = JSON.parse(gh("api", `repos/${repo}/actions/artifacts?per_page=100&name=resource-figures`)).artifacts || [];
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "resource-history-"));
	try {
		for (const artifact of listed.filter(item => !item.expired && item.workflow_run?.head_branch === branch).slice(0, Number(limit) || 30)) {
			const target = path.join(dir, String(artifact.id));
			fs.mkdirSync(target);
			gh("run", "download", String(artifact.workflow_run.id), "-R", repo, "-n", "resource-figures", "-D", target);
		}
		return readRuns(dir);
	} finally {
		fs.rmSync(dir, { recursive: true, force: true });
	}
}

const args = parseArgs(process.argv.slice(2));
let runs = args.github ? fromGitHub(args.github, args.limit, args.branch || "main") : args.files.flatMap(readRuns);
if (!runs.length) {
	console.error("usage: node tools/resources/history.mjs <history.jsonl | run.json | folder>... | --github <owner/repo>");
	process.exit(2);
}
runs.sort((a, b) => String(a.date).localeCompare(String(b.date)));
const keys = args.metric ? args.metric.split(",") : METRICS.filter(metric => metric.gate && metric.key !== "leftRunning").map(metric => metric.key);
const ids = args.workload ? args.workload.split(",") : [...new Set(runs.flatMap(run => Object.keys(run.workloads)))];
for (const id of ids) {
	const rows = runs.filter(run => run.workloads[id]);
	if (!rows.length) continue;
	const header = ["date", "commit", "platform", ...keys.map(key => METRIC[key]?.label || key)];
	const table = rows.map((run, index) => {
		const previous = rows[index - 1]?.workloads[id];
		return [String(run.date).slice(0, 16).replace("T", " "), String(run.commit || "?").slice(0, 7), run.platform || "?", ...keys.map(key => {
			const value = run.workloads[id][key];
			const before = previous?.[key];
			// a move of more than a quarter either way is marked, so a step stands out in a long table
			const mark = before && value != null && Math.abs(value / before - 1) > 0.25 ? (value > before ? " ↑" : " ↓") : "";
			return `${format(key, value)}${mark}`;
		})];
	});
	const widths = header.map((cell, column) => Math.max(cell.length, ...table.map(row => row[column].length)));
	console.log(`\n${id}`);
	console.log(header.map((cell, column) => cell.padEnd(widths[column])).join("  "));
	for (const row of table) console.log(row.map((cell, column) => (column < 3 ? cell.padEnd(widths[column]) : cell.padStart(widths[column]))).join("  "));
}
