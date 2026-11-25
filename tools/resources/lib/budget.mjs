// Checks a run's figures against a baseline (tools/resources/baseline.json, or another run) and writes new baselines.
//
// baseline.json:
//   { "version": 1,
//     "tolerances": { "<metric>": { "ratio": 1.5, "slack": 8388608, "gate": true } },          // over metrics.mjs's defaults
//     "platforms": { "win32-x64": { "<workload>": {
//         "recorded": { "commit": "...", "date": "YYYY-MM-DD", "where": "...", "runs": 3 },
//         "metrics": { "<metric>": <median of the runs> },
//         "spread": { "<metric>": <largest run / smallest run> },
//         "tolerances": { "<metric>": { ... } } } } } }                                       // this workload only
//
// A gated metric is over budget when it exceeds baseline * ratio + slack. It is "better" when it is under
// (baseline - slack) / ratio: nothing fails, but the baseline should come down with it, in the same commit as the change.

import fs from "node:fs";
import { METRICS, METRIC, tolerance } from "./metrics.mjs";

export function platformKey() {
	return `${process.platform}-${process.arch}`;
}

export function readBaseline(file) {
	if (!fs.existsSync(file)) return { version: 1, tolerances: {}, platforms: {} };
	return JSON.parse(fs.readFileSync(file, "utf8"));
}

// The figures `run` (a measure.mjs result) holds for workload `id`, in the shape a baseline holds them
export function figures(result) {
	const out = {};
	for (const metric of METRICS) {
		const value = result.metrics[metric.key];
		if (value != null && !Number.isNaN(value)) out[metric.key] = value;
	}
	return out;
}

// One row per metric: { key, baseline, value, budget, verdict } with verdict one of
// "over" (fails), "ok", "better" (under the baseline by more than its tolerance), "report" (never gated), "new" (no
// baseline), "missing" (this platform cannot measure it)
export function check(result, base, overrides = {}) {
	const rows = [];
	for (const metric of METRICS) {
		const value = result.metrics[metric.key];
		const baseline = base?.metrics?.[metric.key];
		const limits = tolerance(metric.key, { ...overrides, ...(base?.tolerances || {}) });
		const row = { key: metric.key, value, baseline, gate: limits.gate, ratio: limits.ratio, slack: limits.slack, spread: base?.spread?.[metric.key] };
		if (value == null || Number.isNaN(value)) row.verdict = "missing";
		else if (baseline == null) row.verdict = "new";
		else {
			row.budget = baseline * limits.ratio + limits.slack;
			const floor = (baseline - limits.slack) / limits.ratio;
			if (!limits.gate) row.verdict = "report";
			else if (value > row.budget) row.verdict = "over";
			else if (value < floor) row.verdict = "better";
			else row.verdict = "ok";
		}
		rows.push(row);
	}
	return rows;
}

// The workload's verdict: "fail" (a gated metric over budget, or the command failed), "better", "ok" or "new"
export function verdict(rows, result) {
	if (result.exitCode !== 0) return "fail";
	if (rows.some(row => row.verdict === "over")) return "fail";
	if (rows.every(row => row.verdict === "new" || row.verdict === "missing")) return "new";
	if (rows.some(row => row.verdict === "better")) return "better";
	return "ok";
}

function median(values) {
	const sorted = [...values].sort((a, b) => a - b);
	const mid = Math.floor(sorted.length / 2);
	return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

// A baseline entry from several runs of one workload: each metric's median, and its spread (largest / smallest)
export function entryFrom(results, recorded) {
	const metrics = {}, spread = {};
	for (const metric of METRICS) {
		const values = results.map(result => result.metrics[metric.key]).filter(value => value != null && !Number.isNaN(value));
		if (!values.length) continue;
		metrics[metric.key] = Math.round(median(values));
		const low = Math.min(...values), high = Math.max(...values);
		if (values.length > 1) spread[metric.key] = low > 0 ? Number((high / low).toFixed(3)) : high === 0 ? 1 : null;
	}
	return { recorded: { ...recorded, runs: results.length }, metrics, spread };
}

// Writes `entries` ({ workload: entry }) for `platform` into the baseline file, keeping every other platform's,
// workload's and tolerance's entries as they are
export function writeBaseline(file, platform, entries) {
	const baseline = readBaseline(file);
	baseline.version = 1;
	baseline.tolerances ||= {};
	baseline.platforms ||= {};
	const target = (baseline.platforms[platform] ||= {});
	for (const [id, entry] of Object.entries(entries)) {
		// a workload's own tolerances survive a rewrite of its figures
		target[id] = { ...entry, ...(target[id]?.tolerances ? { tolerances: target[id].tolerances } : {}) };
	}
	baseline.platforms[platform] = Object.fromEntries(Object.entries(target).sort(([a], [b]) => a.localeCompare(b)));
	fs.writeFileSync(file, `${JSON.stringify(baseline, null, "\t")}\n`);
	return baseline;
}

// Metrics whose measured spread is wider than their tolerance lets through: such a metric would flake
export function tooNoisy(entry, overrides = {}) {
	const noisy = [];
	for (const [key, spread] of Object.entries(entry.spread || {})) {
		const limits = tolerance(key, { ...overrides, ...(entry.tolerances || {}) });
		const baseline = entry.metrics[key];
		if (!limits.gate || spread == null || !METRIC[key]) continue;
		// the largest run, from the median and the spread, against the budget
		if (baseline * spread > baseline * limits.ratio + limits.slack) noisy.push({ key, spread, ratio: limits.ratio });
	}
	return noisy;
}
