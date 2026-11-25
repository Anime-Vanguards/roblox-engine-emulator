// The plain-language report of a suite run: Markdown for the GitHub job summary, aligned text for a terminal. Each
// workload gets a verdict line, then a row per metric: its name, what it means in one sentence (the technical name
// second), the baseline, this run and the verdict.

import { METRIC, format } from "./metrics.mjs";

const MARK = { fail: "❌", better: "⬇️", ok: "✅", new: "🆕" };

function rowVerdict(row) {
	switch (row.verdict) {
		case "over": return `❌ over budget (up to ${format(row.key, row.budget)})`;
		case "better": return "⬇️ lower than the baseline: lower it too";
		case "ok": return "✅ within budget";
		case "report": return change(row) ? `shown only (${change(row)})` : "shown only";
		case "new": return "no baseline yet";
		default: return "not measured on this platform";
	}
}

function change(row) {
	if (row.baseline == null || row.value == null || !row.baseline) return "";
	const percent = (row.value / row.baseline - 1) * 100;
	if (Math.abs(percent) < 0.5) return "same";
	return `${percent > 0 ? "+" : ""}${percent.toFixed(0)} %`;
}

function folderLine(folder) {
	const parts = [`${folder.label}: ${folder.growth >= 0 ? "+" : ""}${format("diskGrowth", folder.growth)}`, `${folder.created} new files`];
	if (folder.deleted) parts.push(`${folder.deleted} deleted`);
	if (folder.left) parts.push(`${folder.left} left behind (${folder.leftNames.join(", ")}${folder.left > folder.leftNames.length ? ", ..." : ""})`);
	return parts.join(", ");
}

function headline(checked, suite) {
	const failed = checked.filter(item => item.verdict === "fail");
	const better = checked.filter(item => item.verdict === "better");
	const fresh = checked.filter(item => item.verdict === "new");
	const lines = [];
	if (failed.length) {
		lines.push(`❌ ${failed.length} of ${checked.length} workloads went over their budget or failed.`);
		for (const item of failed) {
			if (item.result.exitCode !== 0) lines.push(`- **${item.workload.title}**: the command failed (exit ${item.result.exitCode}); its log is in the run's folder.`);
			for (const row of item.rows.filter(row => row.verdict === "over")) {
				const times = row.baseline ? ` (${(row.value / row.baseline).toFixed(1)}x)` : "";
				lines.push(`- **${item.workload.title}**: ${METRIC[row.key].label.toLowerCase()} ${format(row.key, row.value)} against a baseline of ${format(row.key, row.baseline)}${times}; the budget is ${format(row.key, row.budget)}.`);
			}
		}
		lines.push("", `If the increase is intended, raise the baseline in the same commit as the change, so a reviewer sees it: \`node tools/resources/measure.mjs --suite ${suite} --repeat 3 --write-baseline\` (AGENTS.md, "Resource budgets"). Otherwise find what made the run do more work.`);
	} else lines.push(`✅ All ${checked.length} workloads are within their budgets.`);
	if (better.length) lines.push("", `⬇️ ${better.map(item => item.workload.title).join(", ")} used less than their baseline: lower the baseline with \`--write-baseline\` so the gain is kept.`);
	if (fresh.length) lines.push("", `🆕 ${fresh.map(item => item.workload.title).join(", ")} have no baseline on this platform yet (nothing is checked for them).`);
	return lines;
}

export function markdown({ suite, platform, against, checked }) {
	const lines = [`## Resource budgets: suite \`${suite}\` on ${platform}`, "", ...headline(checked, suite), ""];
	lines.push(`Compared with ${against}. Gated rows fail the run when they go over their budget; "shown only" rows never do.`, "");
	for (const item of checked) {
		const { workload, rows, result } = item;
		lines.push(`### ${MARK[item.verdict] || ""} ${workload.title}`, "", workload.about, "");
		lines.push("| Metric | What it means | Baseline | This run | Verdict |", "|---|---|---:|---:|---|");
		for (const row of rows) {
			if (row.verdict === "missing" && row.baseline == null) continue;
			const metric = METRIC[row.key];
			lines.push(`| **${metric.label}**<br><sub>${metric.technical}</sub> | ${metric.meaning} | ${row.baseline == null ? "-" : format(row.key, row.baseline)} | ${format(row.key, row.value)} | ${rowVerdict(row)} |`);
		}
		lines.push("");
		if (result.folders?.length) lines.push(...result.folders.map(folder => `- ${folderLine(folder)}`), "");
		if (result.leftRunningNames?.length) lines.push(`- Left running and then ended: ${result.leftRunningNames.join(", ")}`, "");
		if (result.perProcess?.length) {
			lines.push("<details><summary>Per program</summary>", "", "| Program | Ran | Biggest memory | Written | Read | Processor time |", "|---|---:|---:|---:|---:|---:|");
			for (const proc of result.perProcess.slice(0, 12)) lines.push(`| ${proc.image} | ${proc.count} | ${format("peakWorkingSetMax", proc.peakWorkingSet)} | ${format("ioWriteBytes", proc.writeBytes)} | ${format("ioReadBytes", proc.readBytes)} | ${format("cpuMs", proc.cpuMs)} |`);
			lines.push("", "</details>", "");
		}
		if (result.missing?.length) lines.push(`Not measured on this platform: ${result.missing.map(key => METRIC[key]?.label || key).join(", ")}.`, "");
	}
	return `${lines.join("\n")}\n`;
}

function pad(text, width, right) {
	text = String(text);
	return right ? text.padStart(width) : text.padEnd(width);
}

export function text({ suite, platform, against, checked }) {
	const lines = [`Resource budgets: suite ${suite} on ${platform}, compared with ${against}`, ""];
	lines.push(...headline(checked, suite).map(line => line.replace(/\*\*/g, "").replace(/`/g, "")), "");
	for (const item of checked) {
		const { workload, rows, result } = item;
		lines.push(`${MARK[item.verdict] || ""} ${workload.title}`, `   ${workload.about}`);
		for (const row of rows) {
			if (row.verdict === "missing" && row.baseline == null) continue;
			const metric = METRIC[row.key];
			lines.push(`   ${pad(metric.label, 36)} ${pad(row.baseline == null ? "-" : format(row.key, row.baseline), 11, true)} -> ${pad(format(row.key, row.value), 11, true)}  ${rowVerdict(row)}`);
		}
		if (result.folders?.length) lines.push(...result.folders.map(folder => `   ${folderLine(folder)}`));
		if (result.leftRunningNames?.length) lines.push(`   Left running and then ended: ${result.leftRunningNames.join(", ")}`);
		lines.push("");
	}
	lines.push("What the rows mean:");
	for (const key of new Set(checked.flatMap(item => item.rows.filter(row => !(row.verdict === "missing" && row.baseline == null)).map(row => row.key)))) {
		lines.push(`   ${METRIC[key].label} (${METRIC[key].technical}): ${METRIC[key].meaning}`);
	}
	return `${lines.join("\n")}\n`;
}
