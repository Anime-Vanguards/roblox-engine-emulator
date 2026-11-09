// Tests of the CI test runner and its entry: node --test tools/ci/test.mjs
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { command } from "../release/common.mjs";
import { OWN_JOBS, jobOf, parseTestList, partition, run } from "./test-runner.mjs";

// Where cargo runs planted's binary (the profile-view package's folder), and the package name it sets there
const cwd = path.resolve("profile-view");
const env = { CARGO_PKG_NAME: "ree-profile-view" };
const planted = path.resolve("target/release/deps/planted-0123abcd.exe");
// Standalone's binary belongs to the engine's package, which cargo runs in the repository's root
const standalone = path.resolve("target/release/deps/standalone-0123abcd.exe");
const engineEnv = { CARGO_PKG_NAME: "roblox-engine-emulator" };
// A folder that is also named profile-view, but belongs to another package
const otherPackage = path.resolve("tools/profile-view");

// A fake spawn that answers every call with `result` and records the arguments it was called with
function fakeSpawn(result) {
	const calls = [];
	const spawn = (file, args) => {
		calls.push(args);
		return result;
	};
	return { calls, spawn };
}

test("the slices of the live list hold every test exactly once, for new and similar names too", () => {
	const names = parseTestList("a::first: test\r\na::first_extended: test\r\nz: test\r\nnew_test: test\r\n");
	for (const count of [1, 2, 3, 4, 7]) {
		const slices = Array.from({ length: count }, (_, index) => partition(names, index, count));
		assert.deepEqual(slices.flat().sort(), names);
		assert.equal(new Set(slices.flat()).size, names.length);
	}
});

test("a list that is empty, repeats a name or does not read as libtest's fails, and so does a bad shard", () => {
	for (const text of ["", "a: test\na: test", "a: test\nunknown", "--skip: test"]) assert.throws(() => parseTestList(text));
	for (const [index, count] of [[-1, 3], [3, 3], [0, 0], [0.5, 3], [0, NaN]]) assert.throws(() => partition([], index, count));
});

test("core skips only the two test binaries with jobs of their own and passes everything else through", () => {
	assert.equal(jobOf(planted, "ree-profile-view"), "planted");
	assert.equal(jobOf(standalone, "roblox-engine-emulator"), "standalone");
	// the same target in another package, each other's package, no package name, and other targets of the two packages
	assert.equal(jobOf(planted, "another-package"), "core");
	assert.equal(jobOf(planted, "roblox-engine-emulator"), "core");
	assert.equal(jobOf(standalone, "ree-profile-view"), "core");
	assert.equal(jobOf(planted, undefined), "core");
	assert.equal(jobOf(standalone, undefined), "core");
	assert.equal(jobOf(path.resolve("target/release/deps/engine-0123abcd.exe"), "ree-profile-view"), "core");
	assert.equal(jobOf(path.resolve("target/release/deps/standalone_more-0123abcd.exe"), "roblox-engine-emulator"), "core");

	for (const [binary, packageEnv] of [[planted, env], [standalone, engineEnv]]) {
		const skipped = fakeSpawn({ status: 23 });
		assert.equal(run(["core", binary], { cwd, env: packageEnv, spawn: skipped.spawn, log() {} }), 0);
		assert.deepEqual(skipped.calls, []);
	}
});

test("the workflow has jobs for both test binaries that core skips", () => {
	const workflow = fs.readFileSync(new URL("../../.github/workflows/ci.yml", import.meta.url), "utf8");
	const modes = [...workflow.matchAll(/runner=\['node','\$entry','(\w+)'/g)].map(match => match[1]);
	assert.deepEqual(modes.sort(), ["core", ...Object.keys(OWN_JOBS)].sort());
});

// The runner knows a test binary by its package and its file's name, and cargo names the file after the target alone.
// A binary, example or bench of the same package with the same name would read as the same: core would skip its tests,
// and the job of its own, which asks cargo for the test target, would never run them.
test("each test binary with a job of its own is a test target, and the only target of that name in its package", () => {
	const { packages } = JSON.parse(command("cargo", ["metadata", "--no-deps", "--format-version", "1", "--locked"]));
	for (const { packageName, target } of Object.values(OWN_JOBS)) {
		const targets = packages.find(candidate => candidate.name === packageName)?.targets ?? [];
		// in a file's name cargo writes a hyphen of the target's name as an underscore
		const kinds = targets.filter(candidate => candidate.name.replaceAll("-", "_") === target).map(candidate => candidate.kind);
		assert.deepEqual(kinds, [["test"]], `the targets of ${packageName} named ${target}`);
	}
});

// A test binary named planted that is not profile-view's: core runs it and returns its exit code, a shard rejects it
function checkRunsInCoreOnly(packageEnv) {
	const passedThrough = fakeSpawn({ status: 23 });
	const options = { cwd: otherPackage, env: packageEnv, spawn: passedThrough.spawn };
	assert.equal(run(["core", planted, "--list", "--format", "terse"], options), 23);
	assert.deepEqual(passedThrough.calls, [["--list", "--format", "terse"]]);
	assert.throws(() => run(["planted", "0", "3", planted], options), /different test target/);
	assert.equal(passedThrough.calls.length, 1);
}

test("core runs the planted binary of another package in a folder also named profile-view, and a shard rejects it", () => {
	checkRunsInCoreOnly({ CARGO_PKG_NAME: "another-package" });
});

test("core runs the planted binary when cargo gives no package name, and a shard rejects it", () => {
	checkRunsInCoreOnly({});
});

test("the standalone job runs its test binary as cargo asked and rejects any other", () => {
	const ran = fakeSpawn({ status: 23 });
	const options = { cwd: path.resolve("."), env: engineEnv, spawn: ran.spawn };
	assert.equal(run(["standalone", standalone, "--list", "--format", "terse"], options), 23);
	assert.deepEqual(ran.calls, [["--list", "--format", "terse"]]);
	// planted's binary, a standalone target of another package or of no known package, and standalone's binary in a shard
	assert.throws(() => run(["standalone", planted], { ...options, env }), /different test target/);
	assert.throws(() => run(["standalone", standalone], { ...options, env: { CARGO_PKG_NAME: "another-package" } }), /different test target/);
	assert.throws(() => run(["standalone", standalone], { ...options, env: {} }), /different test target/);
	assert.throws(() => run(["planted", "0", "3", standalone], options), /different test target/);
	assert.equal(ran.calls.length, 1);
});

test("a shard lists the binary, runs its slice by exact name one test at a time and exits with the tests' code", () => {
	const calls = [];
	const spawn = (file, args, options) => {
		calls.push(args);
		assert.equal(file, planted);
		assert.equal(options.cwd, cwd);
		if (calls.length === 1) return { status: 0, stdout: "a: test\na_extra: test\nb: test\nc: test\n" };
		return { status: 17 };
	};
	assert.equal(run(["planted", "0", "3", planted], { cwd, env, spawn, log() {} }), 17);
	assert.deepEqual(calls, [["--list", "--format", "terse"], ["--exact", "--test-threads=1", "a", "c"]]);
});

test("a failed listing, a signal, a launch error or a bad argument cannot turn a shard green", () => {
	const argv = ["planted", "0", "3", planted];
	assert.equal(run(argv, { cwd, env, spawn: () => ({ status: 7 }) }), 7);
	assert.equal(run(argv, { cwd, env, spawn: () => ({ status: null, signal: "SIGTERM" }) }), 1);
	assert.throws(() => run(argv, { cwd, env, spawn: () => ({ error: new Error("cannot start") }) }), /cannot start/);
	assert.throws(() => run(argv, { cwd, env, spawn: () => ({ status: 0, stdout: "broken list" }) }), /unrecognized/);
	assert.throws(() => run(["planted", "0", "3", path.resolve("target/release/deps/engine-0123abcd.exe")], { cwd, env }), /different test target/);
	assert.throws(() => run(["unknown", planted]), /runner mode/);
	assert.throws(() => run(["core"]), /missing test binary/);
});

test("a shard with no tests runs nothing, and the arguments for the binary come before the names", () => {
	const only = fakeSpawn({ status: 0, stdout: "only: test\n" });
	assert.equal(run(["planted", "2", "3", planted], { cwd, env, spawn: only.spawn, log() {} }), 0);
	assert.equal(only.calls.length, 1);
	assert.equal(run(["planted", "0", "3", planted, "--list", "--format", "terse"], { cwd, env, spawn: only.spawn, log() {} }), 0);
	assert.deepEqual(only.calls[2], ["--exact", "--test-threads=1", "--list", "--format", "terse", "only"]);
});

// node stands in for the test binary, and the entry runs as cargo starts it, as a separate process
test("the entry runs the test binary and exits with its code, whether that is 0 or not", () => {
	const entry = fileURLToPath(new URL("./test-entry.mjs", import.meta.url));
	for (const code of [0, 23]) {
		const script = `console.log("test binary ran"); process.exitCode = ${code};`;
		const result = spawnSync(process.execPath, [entry, "core", process.execPath, "-e", script], { encoding: "utf8", windowsHide: true });
		assert.ifError(result.error);
		assert.equal(result.stdout.trim(), "test binary ran");
		assert.equal(result.stderr, "");
		assert.equal(result.status, code);
	}
});
