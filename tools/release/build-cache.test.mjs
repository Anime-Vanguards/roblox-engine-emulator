// Tests of build-cache.mjs, the key of a release build's dependency cache
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { buildCacheKey, isReleaseCommit, lockWithoutOwnVersions, manifestWithoutVersion } from "./build-cache.mjs";

const SCRIPT = fileURLToPath(new URL("./build-cache.mjs", import.meta.url));
const FILES = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml"];

const manifest = (version, extra = "") => `[package]\nname = "roblox-engine-emulator"\nversion = "${version}"\nedition = "2024"\n\n[dependencies]\nserde = { version = "1.0.228" }\nror-mesh = { git = "https://example.invalid/ror", rev = "abc" }\n${extra}\n[profile.release]\nopt-level = 3\nlto = false\n`;
const lock = (version, rorRev = "abc", serde = "1.0.228") => [
	"version = 4",
	"",
	"[[package]]",
	'name = "ror-mesh"',
	'version = "0.5.0"',
	`source = "git+https://example.invalid/ror?rev=${rorRev}#${rorRev}"`,
	"",
	"[[package]]",
	'name = "serde"',
	`version = "${serde}"`,
	'source = "registry+https://github.com/rust-lang/crates.io-index"',
	'checksum = "00"',
	"",
	"[[package]]",
	'name = "roblox-engine-emulator"',
	`version = "${version}"`,
	"dependencies = [",
	' "ror-mesh",',
	' "serde",',
	"]",
	"",
].join("\n");
const tree = (map) => (file) => (file in map ? map[file] : null);
const key = (map) => buildCacheKey(FILES, tree(map));
const base = { "Cargo.toml": manifest("0.11.0"), "Cargo.lock": lock("0.11.0"), ".cargo/config.toml": '[target.x86_64-pc-windows-msvc]\nlinker = "rust-lld"\n' };

test("a version bump keeps the build cache key", () => {
	const bumped = { ...base, "Cargo.toml": manifest("0.12.0"), "Cargo.lock": lock("0.12.0") };
	assert.equal(key(bumped), key(base));
	assert.match(key(base), /^[0-9a-f]{16}$/);
	const crlf = Object.fromEntries(Object.entries(base).map(([file, text]) => [file, text.replace(/\n/g, "\r\n")]));
	assert.equal(key(crlf), key(base));
});

test("a changed dependency, pin, feature, profile or config changes the key", () => {
	const variants = [
		{ "Cargo.lock": lock("0.11.0", "def") },
		{ "Cargo.lock": lock("0.11.0", "abc", "1.0.229") },
		{ "Cargo.toml": manifest("0.11.0", '[features]\ndefault = ["x"]\n') },
		{ "Cargo.toml": manifest("0.11.0").replace("lto = false", 'lto = "thin"') },
		{ "Cargo.toml": manifest("0.11.0").replace('version = "1.0.228"', 'version = "1.0.229"') },
		{ ".cargo/config.toml": '[target.x86_64-pc-windows-msvc]\nlinker = "link.exe"\n' },
		{ ".cargo/config.toml": null },
	];
	const keys = new Set([key(base), ...variants.map((change) => key({ ...base, ...change }))]);
	assert.equal(keys.size, variants.length + 1);
});

test("only the workspace's own versions are left out", () => {
	const workspace = '[workspace.package]\nversion = "1.2.3"\n\n[workspace.dependencies]\nserde = { version = "1.0.0" }\n[dependencies.extra]\nversion = "2.0.0"\n';
	assert.equal(manifestWithoutVersion(workspace), workspace.replace('version = "1.2.3"', 'version = "0.0.0"'));
	assert.equal(manifestWithoutVersion('[package]\nversion.workspace = true\n'), '[package]\nversion.workspace = true\n');
	const stripped = lockWithoutOwnVersions(lock("0.11.0"));
	assert.match(stripped, /name = "roblox-engine-emulator"\nversion = "0\.0\.0"/);
	assert.match(stripped, /name = "serde"\nversion = "1\.0\.228"/);
	assert.match(stripped, /name = "ror-mesh"\nversion = "0\.5\.0"/);
});

test("pruning leaves only the dependencies' outputs", () => {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ree-build-cache-test-"));
	try {
		const host = path.join(dir, "target/release");
		const target = path.join(dir, "target/x86_64-pc-windows-msvc/release");
		const own = [
			"target/release/build/roblox-engine-emulator-0a1b/build-script-build.exe",
			"target/release/.fingerprint/roblox-engine-emulator-0a1b/build-script",
			"target/x86_64-pc-windows-msvc/release/ree.exe",
			"target/x86_64-pc-windows-msvc/release/ree.d",
			"target/x86_64-pc-windows-msvc/release/incremental/x/y",
			"target/x86_64-pc-windows-msvc/release/deps/ree-1111.exe",
			"target/x86_64-pc-windows-msvc/release/deps/libroblox_engine_emulator-2222.rlib",
			"target/x86_64-pc-windows-msvc/release/build/roblox-engine-emulator-4444/out/gen.rs",
			"target/x86_64-pc-windows-msvc/release/.fingerprint/roblox-engine-emulator-5555/lib-roblox_engine_emulator",
		];
		const kept = [
			"target/release/deps/serde_derive-aaaa.dll",
			"target/release/build/mlua-sys-bbbb/build-script-build.exe",
			"target/release/.fingerprint/mlua-sys-bbbb/build-script",
			"target/x86_64-pc-windows-msvc/release/deps/libserde-cccc.rlib",
			"target/x86_64-pc-windows-msvc/release/build/mlua-sys-eeee/out/lvm.o",
			"target/x86_64-pc-windows-msvc/release/.fingerprint/serde-ffff/lib-serde",
		];
		for (const file of [...own, ...kept]) {
			fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
			fs.writeFileSync(path.join(dir, file), "x");
		}
		const result = spawnSync(process.execPath, [SCRIPT, "--prune", host, target, path.join(dir, "missing"), "--names", "roblox-engine-emulator,ree"], { encoding: "utf8" });
		assert.equal(result.status, 0, result.stderr);
		for (const file of own) assert.ok(!fs.existsSync(path.join(dir, file)), `${file} was left`);
		for (const file of kept) assert.ok(fs.existsSync(path.join(dir, file)), `${file} was deleted`);
		assert.equal(spawnSync(process.execPath, [SCRIPT, "--prune", host], { encoding: "utf8" }).status, 2);
		assert.equal(spawnSync(process.execPath, [SCRIPT, "--prune", "--names", "x"], { encoding: "utf8" }).status, 2);
	} finally {
		fs.rmSync(dir, { recursive: true, force: true });
	}
});

test("the script prints the key of the files it is given", () => {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ree-build-cache-test-"));
	try {
		for (const [file, text] of Object.entries(base)) {
			fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
			fs.writeFileSync(path.join(dir, file), text);
		}
		const result = spawnSync(process.execPath, [SCRIPT, ...FILES], { cwd: dir, encoding: "utf8" });
		assert.equal(result.status, 0, result.stderr);
		assert.equal(result.stdout.trim(), key(base));
		assert.equal(spawnSync(process.execPath, [SCRIPT], { cwd: dir, encoding: "utf8" }).status, 2);
	} finally {
		fs.rmSync(dir, { recursive: true, force: true });
	}
});

test("release-commit detection flags version bumps and fails closed without a parent commit", () => {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ree-release-commit-test-"));
	try {
		const git = (...argv) => {
			const r = spawnSync("git", argv, { cwd: dir, encoding: "utf8" });
			assert.equal(r.status, 0, r.stderr);
		};
		git("init", "--quiet");
		git("config", "user.name", "Release tests");
		git("config", "user.email", "release-test");
		fs.writeFileSync(path.join(dir, "Cargo.toml"), manifest("0.11.0"));
		git("add", "Cargo.toml");
		git("commit", "--quiet", "-m", "Initial");
		// Root commit (no HEAD^): fail closed -> true (clean build)
		assert.equal(isReleaseCommit(dir), true);

		// Ordinary code commit keeping the same version -> false (cache allowed)
		fs.writeFileSync(path.join(dir, "note.txt"), "hi");
		git("add", "note.txt");
		git("commit", "--quiet", "-m", "Regular change");
		assert.equal(isReleaseCommit(dir), false);
		assert.equal(spawnSync(process.execPath, [SCRIPT, "--is-release-commit"], { cwd: dir, encoding: "utf8" }).stdout.trim(), "false");

		// Release version bump commit -> true (clean cacheless build)
		fs.writeFileSync(path.join(dir, "Cargo.toml"), manifest("0.12.0"));
		git("add", "Cargo.toml");
		git("commit", "--quiet", "-m", "Release v0.12.0");
		assert.equal(isReleaseCommit(dir), true);
		assert.equal(spawnSync(process.execPath, [SCRIPT, "--is-release-commit"], { cwd: dir, encoding: "utf8" }).stdout.trim(), "true");
	} finally {
		fs.rmSync(dir, { recursive: true, force: true });
	}
});
