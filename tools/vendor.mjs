// The patched Luau VM and Lune datatypes the engine builds against, vendored in vendor/ so that a
// plain `cargo build` (here, or in a crate that depends on this one) needs nothing else:
//
//   vendor/luau0-src      luau0-src 0.15.9+luau694 (the Luau sources mlua-sys compiles) with patches/luau.patch
//   vendor/lune-roblox    Lune 0.10.4's lune-roblox (Roblox datatypes as Luau values) with patches/lune.patch
//   vendor/lune-utils     its lune-utils, with the same patch
//
// `node tools/vendor.mjs` downloads the pinned archives (SHA-256 checked), applies the patches and
// replaces vendor/ with the result; `--check` rebuilds it in a temporary directory and fails when
// vendor/ differs (a hand edit, or a patch not re-vendored). Change the VM by editing the patch,
// then run this. The patches carry their history from roblox-ui-renderer's tools/native-host.
// The vendored manifests lose `[lints] workspace = true` (Lune's workspace lints, not ours), and each Lune crate
// gets Lune's license text (MPL-2.0, the archive's LICENSE.txt) beside it, since its files are modified copies.
// luau0-src's crate carries its own LICENSE (MIT: Luau's and Lua's notices).
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VENDOR = path.join(ROOT, "vendor");
const CACHE = path.join(ROOT, ".cache", "vendor");

// Windows' own bsdtar: Git Bash puts GNU tar first on PATH, which reads "C:\..." as a remote host
const TAR = process.platform === "win32" && fs.existsSync(path.join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe"))
	? path.join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe") : "tar";

const ARCHIVES = {
	lune: {
		file: "lune-0.10.4.tar.gz",
		url: "https://github.com/lune-org/lune/archive/refs/tags/v0.10.4.tar.gz",
		sha256: "151b7b738b210920297b8afa560f440877662e02a48c64c28ce95baaefc96c95",
	},
	luau: {
		file: "luau0-src.crate",
		url: "https://static.crates.io/crates/luau0-src/luau0-src-0.15.9+luau694.crate",
		sha256: "71a02a835f81c2bbf5edd17b595a7927fa0c91002b11f5bc1d2402d0ac392ed7",
	},
};

const digest = bytes => createHash("sha256").update(bytes).digest("hex");

function run(command, parameters, cwd, input) {
	const result = spawnSync(command, parameters, { cwd, input, stdio: [input === undefined ? "ignore" : "pipe", "inherit", "inherit"], windowsHide: true });
	if (result.error || result.status !== 0) throw result.error || new Error(`${command} ${parameters.join(" ")} exited ${result.status}`);
}

async function archive({ file, url, sha256 }) {
	fs.mkdirSync(CACHE, { recursive: true });
	const target = path.join(CACHE, file);
	if (!fs.existsSync(target) || digest(fs.readFileSync(target)) !== sha256) {
		console.error(`vendor: downloading ${url}`);
		const response = await fetch(url);
		if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
		const bytes = Buffer.from(await response.arrayBuffer());
		if (digest(bytes) !== sha256) throw new Error(`${file}: checksum mismatch`);
		fs.writeFileSync(target, bytes);
	}
	return target;
}

function applyPatch(name, cwd) {
	const input = fs.readFileSync(path.join(ROOT, "patches", name), "utf8").replaceAll("\r\n", "\n");
	// autocrlf off: with core.autocrlf=true (Git for Windows' default) git apply rewrites every file it patches to CRLF,
	// and the archives are LF, so --check failed on any Windows checkout
	run("git", ["-c", "core.autocrlf=false", "apply", "--whitespace=nowarn", "-"], cwd, input);
}

function stripWorkspaceLints(manifest) {
	const text = fs.readFileSync(manifest, "utf8");
	fs.writeFileSync(manifest, text.replace(/\n\[lints\]\nworkspace = true\n/, "\n"));
}

// The vendored tree, built in `work`
async function build(work) {
	const lune = await archive(ARCHIVES.lune);
	const luau = await archive(ARCHIVES.luau);
	fs.mkdirSync(path.join(work, "luau0-src"), { recursive: true });
	run(TAR, ["-xzf", lune, "-C", work], work);
	run(TAR, ["-xzf", luau, "--strip-components=1", "-C", path.join(work, "luau0-src")], work);
	applyPatch("lune.patch", path.join(work, "lune-0.10.4"));
	applyPatch("luau.patch", path.join(work, "luau0-src"));
	const out = path.join(work, "vendor");
	fs.mkdirSync(out, { recursive: true });
	fs.cpSync(path.join(work, "luau0-src"), path.join(out, "luau0-src"), { recursive: true });
	for (const crate of ["lune-roblox", "lune-utils"]) {
		fs.cpSync(path.join(work, "lune-0.10.4", "crates", crate), path.join(out, crate), { recursive: true });
		stripWorkspaceLints(path.join(out, crate, "Cargo.toml"));
		fs.copyFileSync(path.join(work, "lune-0.10.4", "LICENSE.txt"), path.join(out, crate, "LICENSE.txt"));
	}
	return out;
}

function files(directory, base = directory) {
	return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
		const full = path.join(directory, entry.name);
		return entry.isDirectory() ? files(full, base) : [path.relative(base, full).replaceAll("\\", "/")];
	}).sort();
}

const work = fs.mkdtempSync(path.join(os.tmpdir(), "ree-vendor-"));
try {
	const built = await build(work);
	if (process.argv.includes("--check")) {
		// Not compared: vendor/README.md (notes kept across rebuilds) and luau0-src/Cargo.lock, which that crate's own
		// .gitignore keeps out of a checkout, so a fresh clone has none
		const unchecked = new Set(["README.md", "luau0-src/Cargo.lock"]);
		const want = files(built).filter(name => !unchecked.has(name));
		const have = fs.existsSync(VENDOR) ? files(VENDOR).filter(name => !unchecked.has(name)) : [];
		const differing = [...new Set([...want, ...have])].filter(name => {
			const a = path.join(built, name);
			const b = path.join(VENDOR, name);
			return !fs.existsSync(a) || !fs.existsSync(b) || digest(fs.readFileSync(a)) !== digest(fs.readFileSync(b));
		});
		if (differing.length) {
			console.error(`vendor/ differs from the archives with the patches in ${differing.length} files: ${differing.slice(0, 8).join(", ")}${differing.length > 8 ? ", ..." : ""}`);
			process.exitCode = 1; // not process.exit: the temp folder below still has to go
		} else {
			console.log(`vendor/ is the pinned archives with the patches (${want.length} files)`);
		}
	} else {
		const readme = path.join(VENDOR, "README.md");
		const notes = fs.existsSync(readme) ? fs.readFileSync(readme) : null;
		fs.rmSync(VENDOR, { recursive: true, force: true });
		fs.cpSync(built, VENDOR, { recursive: true });
		if (notes) fs.writeFileSync(readme, notes);
		console.log(`vendor/ rebuilt: ${files(VENDOR).length} files`);
	}
} finally {
	fs.rmSync(work, { recursive: true, force: true });
}
