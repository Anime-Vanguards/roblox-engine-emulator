// What a run did to the folders it uses: files created and deleted, growth in bytes, temporary files left behind, and
// (sampled while it runs) the most extra space it took at once.
//
// A folder is watched with a kind: "state" (caches the tool keeps between runs, such as a module store), "out" (what
// the run was asked to write) or "temp" (its temporary folder: anything still there afterwards is left behind). In
// every kind, a `*.tmp` file still there afterwards counts as left behind too.

import fs from "node:fs";
import path from "node:path";

// Every file under `dir` (relative path -> size); an absent folder is empty
export function snapshot(dir) {
	const files = new Map();
	const walk = (folder, prefix) => {
		let entries;
		try {
			entries = fs.readdirSync(folder, { withFileTypes: true });
		} catch {
			return;
		}
		for (const entry of entries) {
			const full = path.join(folder, entry.name);
			const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
			if (entry.isDirectory()) walk(full, rel);
			else if (entry.isFile()) {
				try {
					files.set(rel, fs.statSync(full).size);
				} catch {
					// deleted while walking
				}
			}
		}
	};
	walk(dir, "");
	return files;
}

export function totalBytes(files) {
	let total = 0;
	for (const size of files.values()) total += size;
	return total;
}

// How `after` differs from `before` for a folder of `kind`
export function compare(before, after, kind) {
	let created = 0, deleted = 0, changed = 0, left = 0;
	const leftNames = [];
	for (const [rel, size] of after) {
		const old = before.get(rel);
		if (old === undefined) created++;
		else if (old !== size) changed++;
		const isTemp = kind === "temp" ? old === undefined : rel.endsWith(".tmp") && old === undefined;
		if (isTemp) {
			left++;
			if (leftNames.length < 10) leftNames.push(rel);
		}
	}
	for (const rel of before.keys()) if (!after.has(rel)) deleted++;
	return {
		created, deleted, changed, left, leftNames,
		bytesBefore: totalBytes(before), bytesAfter: totalBytes(after),
		growth: totalBytes(after) - totalBytes(before),
		filesAfter: after.size,
	};
}

// Watches `folders` ([{ label, dir, kind }]) around a run: start() before it, sample() now and then while it runs, finish()
// after it. finish() gives the totals the metrics use and a line per folder.
export function watch(folders) {
	const before = new Map();
	let baseBytes = 0, peakGrowth = 0;
	return {
		start() {
			baseBytes = 0;
			for (const folder of folders) {
				const files = snapshot(folder.dir);
				before.set(folder.dir, files);
				baseBytes += totalBytes(files);
			}
		},
		// the summed size now; cheap enough for the small folders a suite uses
		sample() {
			let bytes = 0;
			for (const folder of folders) bytes += totalBytes(snapshot(folder.dir));
			peakGrowth = Math.max(peakGrowth, bytes - baseBytes);
		},
		finish() {
			const perFolder = [];
			const totals = { filesCreated: 0, filesDeleted: 0, filesLeft: 0, diskGrowth: 0, peakDiskGrowth: 0 };
			let endBytes = 0;
			for (const folder of folders) {
				const after = snapshot(folder.dir);
				const result = compare(before.get(folder.dir) || new Map(), after, folder.kind);
				endBytes += result.bytesAfter;
				perFolder.push({ label: folder.label, kind: folder.kind, ...result });
				totals.filesCreated += result.created;
				totals.filesDeleted += result.deleted;
				totals.filesLeft += result.left;
				totals.diskGrowth += result.growth;
			}
			totals.peakDiskGrowth = Math.max(peakGrowth, endBytes - baseBytes, 0);
			return { totals, perFolder };
		},
	};
}
