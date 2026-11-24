// The release build cache's helpers: its key, and what it leaves out.
//
//   node tools/release/build-cache.mjs <file>...
//       prints the key: 16 hexadecimal digits, a hash of the files that decide how the dependencies are compiled
//       with the repository's own crate versions left out. A version bump changes only those, so a release commit
//       finds the dependencies the dry runs before it compiled. Files named Cargo.toml lose the `version` of their
//       [package] and [workspace.package] tables; files named Cargo.lock lose the version of every package without a
//       `source`, which are the workspace's own. Every other line, and every other file, counts as written (line
//       endings aside), so a changed dependency, feature, profile or pin gives another key. A file that does not exist
//       counts as missing.
//   node tools/release/build-cache.mjs --prune <profile folder>... --names <name>,...
//       deletes the repository's own outputs from Cargo profile folders (target/<profile>,
//       target/<triple>/<profile>) before they are stored, because every build compiles them again: every file at
//       the top of the folder (the binaries Cargo copies there, and anything a step wrote beside them), its
//       incremental/ and examples/ folders, and in deps/, build/ and .fingerprint/ whatever belongs to a named crate,
//       package or test target. A name counts with `-` and with `_`. What is left is the compiled dependencies.
//
// Node.js only, no npm packages; the same file in each tool repository.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { packageFields } from "./common.mjs";

const OWN_VERSION = "0.0.0";

// A Cargo.toml without the package's own version
export function manifestWithoutVersion(text) {
	let table = "";
	return text.split("\n").map((line) => {
		const header = /^\s*\[([^\]]+)\]\s*(?:#.*)?$/.exec(line);
		if (header) table = header[1].trim();
		else if ((table === "package" || table === "workspace.package") && /^\s*version\s*=\s*"[^"]*"/.test(line)) {
			return line.replace(/"[^"]*"/, `"${OWN_VERSION}"`);
		}
		return line;
	}).join("\n");
}

// A Cargo.lock without the versions of the workspace's own packages (the ones with no `source`)
export function lockWithoutOwnVersions(text) {
	return text.split(/(?=^\[\[package\]\]$)/m).map((block) => {
		if (!block.startsWith("[[package]]") || /^source\s*=/m.test(block)) return block;
		return block.replace(/^version\s*=\s*"[^"]*"/m, `version = "${OWN_VERSION}"`);
	}).join("");
}

// The key for these files; `read(file)` gives a file's text, or null when it does not exist
export function buildCacheKey(files, read) {
	const hash = createHash("sha256");
	for (const file of files) {
		const raw = read(file);
		let text = raw === null ? null : raw.replace(/\r\n/g, "\n");
		const name = path.basename(file);
		if (text !== null && name === "Cargo.toml") text = manifestWithoutVersion(text);
		if (text !== null && name === "Cargo.lock") text = lockWithoutOwnVersions(text);
		hash.update(`${file}\0${text === null ? "\u0001missing" : text}\0`);
	}
	return hash.digest("hex").slice(0, 16);
}

// Deletes the outputs of the named crates from each profile folder; returns the paths deleted
export function prune(folders, names) {
	const prefixes = [...new Set(names.flatMap((name) => [name, name.replace(/-/g, "_"), name.replace(/_/g, "-")]))].flatMap((name) => [`${name}-`, `lib${name}-`]);
	const own = (entry) => prefixes.some((prefix) => entry.startsWith(prefix));
	const deleted = [];
	const remove = (target) => {
		fs.rmSync(target, { recursive: true, force: true });
		deleted.push(target);
	};
	for (const folder of folders) {
		if (!fs.existsSync(folder)) continue;
		for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
			const target = path.join(folder, entry.name);
			if (!entry.isDirectory() || entry.name === "incremental" || entry.name === "examples") remove(target);
			else if (entry.name === "deps" || entry.name === "build" || entry.name === ".fingerprint") {
				for (const inner of fs.readdirSync(target)) if (own(inner)) remove(path.join(target, inner));
			}
		}
	}
	return deleted;
}

function readOrNull(file) {
	try {
		return fs.readFileSync(file, "utf8");
	} catch (error) {
		if (error.code === "ENOENT") return null;
		throw error;
	}
}

// Started as the script, compared by real path so that a start through a linked folder counts too
function startedAsScript() {
	try {
		return Boolean(process.argv[1]) && fs.realpathSync.native(process.argv[1]) === fs.realpathSync.native(fileURLToPath(import.meta.url));
	} catch {
		return false;
	}
}

// Whether `HEAD` changed Cargo.toml's [package].version compared to `HEAD^` (e.g. a "Release vX.Y.Z" commit).
// Fails closed to true (clean build, no cache) when the parent commit or manifest cannot be read.
export function isReleaseCommit(root = process.cwd(), git = (args) => execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })) {
	try {
		const current = packageFields(fs.readFileSync(path.join(root, "Cargo.toml"), "utf8")).version;
		const previous = packageFields(git(["show", "HEAD^:Cargo.toml"])).version;
		if (!current || !previous) return true;
		return current !== previous;
	} catch {
		return true;
	}
}

const USAGE = "usage: build-cache.mjs <file>...\n       build-cache.mjs --prune <profile folder>... --names <name>,...\n       build-cache.mjs --is-release-commit";

if (startedAsScript()) {
	const args = process.argv.slice(2);
	if (args[0] === "--is-release-commit" && args.length === 1) {
		console.log(isReleaseCommit() ? "true" : "false");
	} else if (args[0] === "--prune") {
		const at = args.indexOf("--names");
		const folders = args.slice(1, at === -1 ? undefined : at);
		const names = at === -1 ? [] : (args[at + 1] ?? "").split(",").filter(Boolean);
		if (!folders.length || !names.length || args.length !== at + 2) {
			console.error(USAGE);
			process.exit(2);
		}
		const deleted = prune(folders, names);
		console.log(`Left ${deleted.length} of the repository's own outputs out of the cache`);
	} else if (!args.length || args.some((arg) => arg.startsWith("--"))) {
		console.error(USAGE);
		process.exit(2);
	} else {
		console.log(buildCacheKey(args, readOrNull));
	}
}
