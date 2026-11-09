// Tests of the CI test runner and its entry: node --test tools/ci/test.mjs
import assert from "node:assert/strict";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { isPlanted, parseTestList, partition, run } from "./test-runner.mjs";

// Where cargo runs planted's binary (the profile-view package's folder), and the package name it sets there
const cwd = path.resolve("profile-view");
const env = { CARGO_PKG_NAME: "ree-profile-view" };
const planted = path.resolve("target/release/deps/planted-0123abcd.exe");
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

test("core skips only ree-profile-view's planted test binary and passes everything else through", () => {
	assert.equal(isPlanted(planted, "ree-profile-view"), true);
	assert.equal(isPlanted(planted, "another-package"), false);
	assert.equal(isPlanted(planted, undefined), false);
	assert.equal(isPlanted(path.resolve("target/release/deps/engine-0123abcd.exe"), "ree-profile-view"), false);

	const skipped = fakeSpawn({ status: 23 });
	assert.equal(run(["core", planted], { cwd, env, spawn: skipped.spawn, log() {} }), 0);
	assert.deepEqual(skipped.calls, []);
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
