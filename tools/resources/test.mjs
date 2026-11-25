// Tests of the resource-budget tools: the budget arithmetic, the folder accounting, the report, the history table, the
// baseline file, and (on Windows and Linux) a real measurement of a small process tree.
//
//   node --test tools/resources/test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { check, entryFrom, readBaseline, tooNoisy, verdict, writeBaseline } from "./lib/budget.mjs";
import { compare, watch } from "./lib/dirs.mjs";
import { METRIC, METRICS, format } from "./lib/metrics.mjs";
import { markdown, text } from "./lib/report.mjs";
import { measure } from "./lib/tree.mjs";
import { SUITES, workloads } from "./suite.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const temp = prefix => fs.mkdtempSync(path.join(os.tmpdir(), `resources-test-${prefix}-`));

const result = metrics => ({ exitCode: 0, metrics });

test("a gated metric fails over baseline * ratio + slack, and is flagged better under (baseline - slack) / ratio", () => {
	const base = { metrics: { ioWriteBytes: 100 << 20, wallMs: 1000 } };
	const row = value => check(result({ ioWriteBytes: value, wallMs: 5000 }), base).find(entry => entry.key === "ioWriteBytes");
	const limit = (100 << 20) * METRIC.ioWriteBytes.ratio + METRIC.ioWriteBytes.slack;
	assert.equal(row(limit).verdict, "ok");
	assert.equal(row(limit + 1).verdict, "over");
	assert.equal(row(200 << 20).verdict, "over", "a doubling always fails");
	assert.equal(row(10 << 20).verdict, "better");
	// times are shown, never gated, whatever they do
	const wall = check(result({ ioWriteBytes: 100 << 20, wallMs: 50000 }), base).find(entry => entry.key === "wallMs");
	assert.equal(wall.verdict, "report");
	assert.equal(check(result({ ioWriteBytes: 1 }), null).find(entry => entry.key === "ioWriteBytes").verdict, "new");
	assert.equal(check(result({}), base).find(entry => entry.key === "ioWriteBytes").verdict, "missing");
});

test("every gated metric's ratio is under 2, so a doubling of any baseline over twice its slack fails", () => {
	for (const metric of METRICS.filter(entry => entry.gate)) {
		assert.ok(metric.ratio < 2 || metric.slack === 0, metric.key);
		assert.ok(metric.meaning.length > 40 && /\.$/.test(metric.meaning), `${metric.key} says what it means in a sentence`);
	}
});

test("baseline tolerances override the defaults, a workload's own over the file's", () => {
	const base = { metrics: { ioWriteBytes: 100 }, tolerances: { ioWriteBytes: { ratio: 1.1, slack: 0 } } };
	assert.equal(check(result({ ioWriteBytes: 115 }), base, { ioWriteBytes: { ratio: 3, slack: 0 } }).find(entry => entry.key === "ioWriteBytes").verdict, "over");
	assert.equal(check(result({ ioWriteBytes: 115 }), { metrics: { ioWriteBytes: 100 } }, { ioWriteBytes: { ratio: 1.2, slack: 0 } }).find(entry => entry.key === "ioWriteBytes").verdict, "ok");
	assert.equal(check(result({ ioWriteBytes: 115 }), { metrics: { ioWriteBytes: 100 } }, { ioWriteBytes: { gate: false } }).find(entry => entry.key === "ioWriteBytes").verdict, "report");
});

test("a workload fails when its command fails, whatever its figures", () => {
	const rows = check(result({ ioWriteBytes: 1 }), { metrics: { ioWriteBytes: 1 } });
	assert.equal(verdict(rows, { exitCode: 0 }), "ok");
	assert.equal(verdict(rows, { exitCode: 3 }), "fail");
});

test("a baseline entry holds the medians and the spread of its runs, and a rewrite keeps other entries", () => {
	const entry = entryFrom([result({ ioWriteBytes: 100, pageFaults: 10 }), result({ ioWriteBytes: 130, pageFaults: 10 }), result({ ioWriteBytes: 110, pageFaults: 10 })], { commit: "abc", date: "2026-01-01", where: "here" });
	assert.equal(entry.metrics.ioWriteBytes, 110);
	assert.equal(entry.spread.ioWriteBytes, 1.3);
	assert.equal(entry.recorded.runs, 3);
	assert.deepEqual(tooNoisy({ ...entry, metrics: { ioWriteBytes: 100 << 20 }, spread: { ioWriteBytes: 1.9 } }).map(noisy => noisy.key), ["ioWriteBytes"]);

	const dir = temp("baseline");
	const file = path.join(dir, "baseline.json");
	writeBaseline(file, "linux-x64", { keep: { metrics: { ioWriteBytes: 5 }, tolerances: { ioWriteBytes: { ratio: 1.2 } } } });
	writeBaseline(file, "win32-x64", { other: { metrics: { ioWriteBytes: 7 } } });
	writeBaseline(file, "linux-x64", { keep: { metrics: { ioWriteBytes: 6 } }, added: { metrics: { ioWriteBytes: 1 } } });
	const written = readBaseline(file);
	assert.deepEqual(written.platforms["linux-x64"].keep, { metrics: { ioWriteBytes: 6 }, tolerances: { ioWriteBytes: { ratio: 1.2 } } });
	assert.equal(written.platforms["win32-x64"].other.metrics.ioWriteBytes, 7);
	assert.deepEqual(Object.keys(written.platforms["linux-x64"]), ["added", "keep"]);
	fs.rmSync(dir, { recursive: true, force: true });
});

test("folder accounting counts new, changed and deleted files, and what a run left behind", () => {
	const before = new Map([["keep.bin", 10], ["gone.bin", 5], ["old.tmp", 1]]);
	const after = new Map([["keep.bin", 12], ["new.bin", 100], ["stale.tmp", 3], ["old.tmp", 1]]);
	const state = compare(before, after, "state");
	assert.deepEqual({ created: state.created, deleted: state.deleted, changed: state.changed, left: state.left, growth: state.growth }, { created: 2, deleted: 1, changed: 1, left: 1, growth: 100 });
	assert.deepEqual(state.leftNames, ["stale.tmp"]);
	// in a temp folder every new file is left behind
	assert.equal(compare(before, after, "temp").left, 2);

	const dir = temp("folders");
	const folders = watch([{ label: "state", dir, kind: "state" }]);
	folders.start();
	fs.writeFileSync(path.join(dir, "a.bin"), Buffer.alloc(4096));
	folders.sample();
	fs.rmSync(path.join(dir, "a.bin"));
	fs.writeFileSync(path.join(dir, "b.tmp"), "x");
	const { totals } = folders.finish();
	assert.equal(totals.diskGrowth, 1);
	assert.equal(totals.peakDiskGrowth, 4096, "the sampled peak keeps the file deleted before the end");
	assert.equal(totals.filesLeft, 1);
	fs.rmSync(dir, { recursive: true, force: true });
});

test("the report says what each figure means, the budget it broke and how to raise a baseline", () => {
	const workload = { id: "w", title: "A workload", about: "Does a thing." };
	const res = { exitCode: 0, metrics: { ioWriteBytes: 3 * 2 ** 30, pageFaults: 10 }, folders: [{ label: "State folder", growth: 1 << 20, created: 3, deleted: 0, left: 1, leftNames: ["x.tmp"] }] };
	const rows = check(res, { metrics: { ioWriteBytes: 100 << 20, pageFaults: 10 } });
	const report = { suite: "ci", platform: "win32-x64", against: "the baseline", checked: [{ workload, rows, result: res, verdict: verdict(rows, res) }] };
	const md = markdown(report);
	assert.match(md, /❌ 1 of 1 workloads went over their budget/);
	assert.match(md, /data written 3\.00 GB against a baseline of 100 MB \(30\.7x\); the budget is 158 MB/);
	assert.match(md, /Large amounts wear the SSD/);
	assert.match(md, /--write-baseline/);
	assert.match(md, /1 left behind \(x\.tmp\)/);
	assert.match(text(report), /Data written\s+100 MB ->\s+3\.00 GB/);
});

test("figures read as people read them", () => {
	assert.equal(format("ioWriteBytes", 24.8 * 1024 ** 3), "24.8 GB");
	assert.equal(format("ioWriteBytes", 1500), "1.46 KB");
	assert.equal(format("pageFaults", 1234567), "1,234,567");
	assert.equal(format("wallMs", 61500), "61.5 s");
	assert.equal(format("wallMs", 250), "250 ms");
});

test("history prints a table per workload, oldest first, marking steps of more than a quarter", () => {
	const dir = temp("history");
	const file = path.join(dir, "history.jsonl");
	fs.writeFileSync(file, [
		{ date: "2026-01-02T00:00:00Z", commit: "bbbbbbbbb", platform: "win32-x64", workloads: { cold: { ioWriteBytes: 300 << 20 } } },
		{ date: "2026-01-01T00:00:00Z", commit: "aaaaaaaaa", platform: "win32-x64", workloads: { cold: { ioWriteBytes: 100 << 20 } } },
	].map(line => JSON.stringify(line)).join("\n"));
	const printed = spawnSync(process.execPath, [path.join(HERE, "history.mjs"), file, "--metric", "ioWriteBytes"], { encoding: "utf8" });
	assert.equal(printed.status, 0, printed.stderr);
	const lines = printed.stdout.trim().split("\n");
	assert.equal(lines[0], "cold");
	assert.match(lines[2], /^2026-01-01 00:00\s+aaaaaaa .* 100 MB$/);
	assert.match(lines[3], /^2026-01-02 00:00\s+bbbbbbb .* 300 MB ↑$/);
	fs.rmSync(dir, { recursive: true, force: true });
});

test("baseline.json names only workloads and metrics that exist, and covers the ci suite on Windows", () => {
	const baseline = readBaseline(path.join(HERE, "baseline.json"));
	const ids = new Set(workloads({ args: {} }).map(workload => workload.id));
	for (const key of Object.keys(baseline.tolerances || {})) assert.ok(METRIC[key], `tolerance for unknown metric ${key}`);
	for (const [platform, entries] of Object.entries(baseline.platforms || {})) {
		for (const [id, entry] of Object.entries(entries)) {
			assert.ok(ids.has(id), `${platform}: unknown workload ${id}`);
			for (const key of [...Object.keys(entry.metrics), ...Object.keys(entry.tolerances || {})]) assert.ok(METRIC[key], `${platform} ${id}: unknown metric ${key}`);
		}
	}
	for (const id of SUITES.ci.workloads) assert.ok(baseline.platforms?.["win32-x64"]?.[id], `win32-x64 has no baseline for ${id}`);
});

test("a measured process tree reports its processes, the data they wrote and their memory", { skip: !["win32", "linux"].includes(process.platform) }, async () => {
	const dir = temp("tree");
	const script = path.join(dir, "tree.mjs");
	fs.writeFileSync(script, [
		"import fs from 'node:fs';",
		"import { spawnSync } from 'node:child_process';",
		"const [dir, role] = process.argv.slice(2);",
		"fs.writeFileSync(`${dir}/${role || 'root'}.bin`, Buffer.alloc((role ? 1 : 3) << 20, 1));",
		"if (!role) { const child = spawnSync(process.execPath, [process.argv[1], dir, 'child']); process.exit(child.status === 0 ? 7 : 1); }",
	].join("\n"));
	const measured = await measure({ argv: [process.execPath, script, dir], cwd: dir, env: process.env, log: path.join(dir, "log.txt"), toolsDir: path.join(dir, "tools") });
	assert.equal(measured.exitCode, 7, "the command's exit code comes through");
	assert.ok(measured.ioWriteBytes >= 4 << 20, `wrote ${measured.ioWriteBytes}`);
	assert.ok(measured.processes >= 2, `${measured.processes} processes`);
	assert.ok(measured.peakWorkingSetMax > 10 << 20, `peak ${measured.peakWorkingSetMax}`);
	assert.ok(measured.pageFaults > 0 && measured.cpuMs > 0);
	assert.equal(measured.leftRunning, 0);
	fs.rmSync(dir, { recursive: true, force: true });
});
