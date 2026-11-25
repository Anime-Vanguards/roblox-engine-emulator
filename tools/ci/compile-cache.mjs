// The compile cache of the CI and release jobs (.github/workflows/ci.yml, .github/workflows/release.yml):
// sccache between cargo and the compilers across Windows, Linux and macOS.
//
//   node tools/ci/compile-cache.mjs start --tools <folder> --cache <folder> [--prefix <name>] [--workflow <file>] [--size <limit>]
//   node tools/ci/compile-cache.mjs stop
//
// The dependency cache the jobs restore (Swatinem/rust-cache or build-cache.mjs) keeps what the crates.io crates
// compile to, but nothing of vendor/. Cargo decides by modification times whether a path dependency is fresh, and a
// checkout makes every file new, so each job compiled the Luau VM's C++ and the vendored crates again. sccache decides
// by content instead: it stores what a compile wrote under a hash of the compiler, its arguments, its environment and
// every file it read, and hands that back when the same compile is asked for again. A change in vendor/ changes the
// hash, so it always compiles.
//
// `start` downloads the pinned sccache release for this platform into --tools, checks its SHA-256 and gives the job's
// later steps this environment (GITHUB_ENV). A download that fails is tried again a few times, and a job never builds
// without the cache:
//
//   RUSTC_WRAPPER       cargo starts rustc through sccache. sccache answers for library crates only. Whatever is linked
//                       (a binary, a test binary, a build script, a procedural macro) is always built, since sccache
//                       cannot see what the linker read
//   CARGO_INCREMENTAL   0, as the toolchain step sets it already. The release profile asks for incremental builds,
//                       and sccache answers for none of those
//   CC, CXX, INCLUDE    the cc crate compiles the C/C++ of the vendored VM (mlua-sys), mimalloc and zstd. It puts
//                       sccache in front of the compiler only when the environment names the compiler: one it finds by
//                       itself runs unwrapped. Windows names Visual Studio's default cl.exe with vcvars64's INCLUDE;
//                       Linux names cc and c++; macOS names clang and clang++.
//   SCCACHE_DIR, ...    the cache is the folder --cache alone. The workflow restores it before the build and stores it
//                       after a run on main. sccache is never given the workflow's token
//   SCCACHE_DIRECT      false: a C/C++ compile is always looked up by what the preprocessor made of it (see `start`)
//
// and this output (GITHUB_OUTPUT):
//
//   key                 the name the workflow stores the folder under: a hash of everything the vendored builds are made
//                       from. A run that finds its key stored has nothing new to store
//
// `stop` prints what the cache did in this job and stops sccache's server, which holds the folder open until then.
// A job in which no server is running has none to stop, and `stop` says so.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { download as fetchBytes } from "../download.mjs";
import { ROOT, args, command, isMain, main } from "../release/common.mjs";

// The pinned releases (https://github.com/mozilla/sccache/releases/tag/v0.19.0) and each archive's SHA-256
export const SCCACHE_PINS = {
	"win32-x64": {
		url: "https://github.com/mozilla/sccache/releases/download/v0.19.0/sccache-v0.19.0-x86_64-pc-windows-msvc.zip",
		sha256: "28be8c61f6b2c89b9eb2869f6af7f78e6f992e78a8abae0ce855fcc129cbeac1",
		exe: "sccache.exe",
		format: "zip",
	},
	"linux-x64": {
		url: "https://github.com/mozilla/sccache/releases/download/v0.19.0/sccache-v0.19.0-x86_64-unknown-linux-musl.tar.gz",
		sha256: "aa73c7270c997894038c0d8f0d297865f57569e1687883322bc431713d4bb9dd",
		exe: "sccache",
		format: "tar.gz",
	},
	"darwin-arm64": {
		url: "https://github.com/mozilla/sccache/releases/download/v0.19.0/sccache-v0.19.0-aarch64-apple-darwin.tar.gz",
		sha256: "a35a0b7d622618774bc1bb8102d72562dfb21e619f88b153b370d7f89bc286af",
		exe: "sccache",
		format: "tar.gz",
	},
};

// What the vendored builds are made from besides the compilers and the workflow file: the vendored sources, the versions
// and features of the crates they depend on, the build settings, and this script. Folders count with every file below them.
export const BASE_KEY_INPUTS = [
	"vendor", "Cargo.lock", "Cargo.toml", "profile-view/Cargo.toml", "tests/host/Cargo.toml", "tests/planted/Cargo.toml", ".cargo/config.toml",
];

// The most the cache folder holds before sccache drops what was used longest ago. Every job waits for the folder's
// download, so it is kept small. One run's whole output, every crates.io crate included, is 144 MB: this holds that
// and, beside it, what the builds before the last changes of vendor/ or the lockfile stored.
const DEFAULT_CACHE_SIZE = "400M";
const DEFAULT_PREFIX = "compile-cache";
const DEFAULT_WORKFLOW = ".github/workflows/ci.yml";

// What sccache says when it is asked to stop a server and none is running (0.19.0, src/commands.rs)
const NO_SERVER = "couldn't connect to server";

const SYSTEM_ROOT = process.env.SystemRoot || "C:\\Windows";

// The pinned sccache archive for (platform, arch), or throws when unsupported
export function sccachePin(platform = process.platform, arch = process.arch) {
	const key = `${platform}-${arch}`;
	const pin = SCCACHE_PINS[key];
	if (!pin) throw new Error(`no pinned sccache release for ${key}`);
	return pin;
}

// `bytes`, unless their SHA-256 is not `sha256`
export function checked(bytes, sha256, name) {
	const actual = createHash("sha256").update(bytes).digest("hex");
	if (actual !== sha256) throw new Error(`${name}: checksum mismatch, got ${actual}`);
	return bytes;
}

// The variables in what cmd's `set` prints, one NAME=value per line
export function parseEnvironment(text) {
	const variables = {};
	for (const line of text.split(/\r?\n/)) {
		const at = line.indexOf("=");
		if (at > 0) variables[line.slice(0, at)] = line.slice(at + 1);
	}
	return variables;
}

// Every file below `folder`, or `folder` itself when it is a file, sorted so that the order is the same on every machine
function files(folder) {
	if (!fs.statSync(folder).isDirectory()) return [folder];
	return fs.readdirSync(folder, { recursive: true, withFileTypes: true })
		.filter(entry => entry.isFile())
		.map(entry => path.join(entry.parentPath, entry.name))
		.sort();
}

// The cache key: a hash of `facts` (the compilers' versions) and of the name and bytes of every file in `inputs`
// under `root`. Line endings are left as they are: vendor/ is checked out byte for byte (.gitattributes).
export function cacheKey(root, inputs, facts, prefix = DEFAULT_PREFIX) {
	const hash = createHash("sha256");
	for (const fact of facts) hash.update(`${fact}\0`);
	for (const input of inputs) {
		for (const file of files(path.join(root, input))) {
			hash.update(`${path.relative(root, file).replaceAll("\\", "/")}\0`);
			hash.update(fs.readFileSync(file));
			hash.update("\0");
		}
	}
	return `${prefix}-${hash.digest("hex").slice(0, 32)}`;
}

// The bytes at `url`, used only when their SHA-256 is `sha256`. Delegates to tools/download.mjs so network errors,
// HTTP 5xx and 429 are retried with exponential backoff, while non-transient 4xx and checksum mismatches fail at once.
export function download({ url, sha256 }, options = {}) {
	return fetchBytes(url, { sha256, name: url, timeout: 30_000, log: console.log, ...options });
}

// Downloads the pinned archive for this platform and unpacks it into `tools` once its checksum matches; returns sccache's path
async function install(tools, pin) {
	const archive = path.join(tools, path.basename(pin.url));
	fs.mkdirSync(tools, { recursive: true });
	fs.writeFileSync(archive, await download(pin));
	// The archive's one folder carries the version in its name and is left out, so sccache's path stays the same from
	// release to release: the dependency cache's key takes in that path.
	if (pin.format === "zip") {
		// Windows' own tar reads ZIP archives; Git Bash's, which may come first on PATH, does not.
		command(path.join(SYSTEM_ROOT, "System32", "tar.exe"), ["-xf", archive, "-C", tools, "--strip-components", "1"]);
	} else {
		command("tar", ["-xzf", archive, "-C", tools, "--strip-components", "1"]);
	}
	return path.join(tools, pin.exe);
}

// What Visual Studio's environment script sets for 64-bit builds with the newest installed C++ tools. The cc crate
// picks its compiler the same way: the newest Visual Studio, and the tools version its default file names.
function developerEnvironment() {
	const vswhere = path.join(process.env["ProgramFiles(x86)"] || "C:\\Program Files (x86)", "Microsoft Visual Studio", "Installer", "vswhere.exe");
	const studio = command(vswhere, ["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"]).trim();
	if (!studio) throw new Error("no Visual Studio with the C++ tools is installed");
	const script = path.join(studio, "VC", "Auxiliary", "Build", "vcvars64.bat");
	// cmd takes the line as it is written here: the script's path in quotes, its banner dropped, then every variable
	const line = `""${script}" >nul && set"`;
	// The script adds its include paths to any that are set already, so it starts without: the answer must be the same
	// whatever ran before, since the caches' keys and cargo's own freshness check take in INCLUDE.
	const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => name.toUpperCase() !== "INCLUDE"));
	return parseEnvironment(command(process.env.ComSpec || path.join(SYSTEM_ROOT, "System32", "cmd.exe"), ["/d", "/s", "/c", line], { windowsVerbatimArguments: true, env }));
}

// Platform compiler env vars and compiler-version facts for the cache key.
function compilerSetup(platform = process.platform) {
	if (platform === "win32") {
		const studio = developerEnvironment();
		const compiler = path.join(studio.VCToolsInstallDir ?? "", "bin", "Hostx64", "x64", "cl.exe");
		if (!studio.INCLUDE || !fs.existsSync(compiler)) throw new Error("Visual Studio's environment names no C++ compiler and include paths");
		return {
			env: { CXX: compiler, INCLUDE: studio.INCLUDE },
			facts: [studio.VCToolsVersion, studio.WindowsSDKVersion],
			summary: `C++ compiler ${compiler}; Windows SDK ${studio.WindowsSDKVersion}`,
		};
	}
	const [cc, cxx] = platform === "darwin" ? ["clang", "clang++"] : ["cc", "c++"];
	const ccVersion = command(cc, ["--version"]).trim();
	const cxxVersion = command(cxx, ["--version"]).trim();
	return {
		env: { CC: cc, CXX: cxx },
		facts: [ccVersion, cxxVersion],
		summary: `C/C++ compilers ${cc} / ${cxx}`,
	};
}

// Appends NAME=value lines to the file GitHub Actions names in the environment variable `variable`
function publish(variable, values) {
	const file = process.env[variable];
	if (!file) throw new Error(`${variable} is not set: this runs as a step of a GitHub Actions job`);
	fs.appendFileSync(file, Object.entries(values).map(([name, value]) => `${name}=${value}\n`).join(""));
}

async function start({ tools, cache, prefix = DEFAULT_PREFIX, workflow = DEFAULT_WORKFLOW, size = DEFAULT_CACHE_SIZE }) {
	if (!tools || !cache) throw new Error("start needs --tools <folder> and --cache <folder>");
	const pin = sccachePin();
	const sccache = await install(path.resolve(tools), pin);
	const compiler = compilerSetup();
	publish("GITHUB_ENV", {
		RUSTC_WRAPPER: sccache,
		CARGO_INCREMENTAL: "0",
		...compiler.env,
		SCCACHE_DIR: path.resolve(cache),
		SCCACHE_CACHE_SIZE: size,
		// sccache looks a C/C++ compile up by the preprocessor's output, which holds every header the compile read. In
		// direct mode it skips the preprocessor and goes by the headers the same source read last time, so a new
		// header that comes first on the include path goes unseen. Keeping it false ensures every header change hits.
		SCCACHE_DIRECT: "false",
		// the server stays up between the job's cargo commands, until `stop`
		SCCACHE_IDLE_TIMEOUT: "0",
	});
	// the compilers are not files of the checkout, so their versions stand in for them, and sccache's for how it hashes
	const inputs = [...BASE_KEY_INPUTS, workflow, "tools/ci/compile-cache.mjs"];
	const facts = [pin.sha256, command("rustc", ["-vV"]), ...compiler.facts];
	const key = cacheKey(ROOT, inputs, facts, prefix);
	publish("GITHUB_OUTPUT", { key });
	console.log(`compile cache: ${key}; ${compiler.summary}`);
}

// Stops sccache's server, which prints what the cache did in this job as it goes. sccache starts its server for a
// compile and for nothing else, so none is running when no compile of the job went through it, or when the server did
// not outlive the step that started it. Neither is a failure of the job: nothing holds the folder open, and only the
// statistics are missing.
export function stop({ env = process.env, spawn = spawnSync, log = console.log } = {}) {
	const sccache = env.RUSTC_WRAPPER;
	if (!sccache) throw new Error("RUSTC_WRAPPER is not set: `start` did not run in this job");
	const result = spawn(sccache, ["--stop-server"], { encoding: "utf8", windowsHide: true });
	if (result.error) throw new Error(`${sccache} --stop-server: ${result.error.message}`);
	const output = result.stderr || result.stdout || "";
	if (result.status === 0) log(result.stdout);
	else if (output.includes(NO_SERVER)) log("compile cache: no sccache server is running, so there is none to stop and there are no statistics. No compile of this job went through sccache, or its server did not outlive the step that started it.");
	else throw new Error(`${sccache} --stop-server: exit ${result.status}: ${output}`);
}

if (isMain(import.meta.url)) {
	await main(async () => {
		const [mode, ...rest] = process.argv.slice(2);
		if (mode === "start") await start(args(rest, ["tools", "cache", "prefix", "workflow", "size"]));
		else if (mode === "stop" && rest.length === 0) stop();
		else throw new Error("usage: compile-cache.mjs start --tools <folder> --cache <folder> [--prefix <name>] [--workflow <file>] [--size <limit>] | stop");
	});
}
