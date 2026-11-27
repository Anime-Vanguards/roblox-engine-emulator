// Installs of the executables the process tests run: ree, and on Windows node itself as the mock asset worker.
//
// Each executable is staged once and every install is a hard link to it. A virus scanner reads every new copy of an
// executable in full (Windows Defender: half a second to two seconds of CPU and of wall time for each copy of ree or node,
// many at once when the checks run side by side), and a hard link to a file it has read is no new file to it.
// tests/standalone.rs stages ree and node for all of its scripts and names the folder in REE_TEST_STAGE; a script run
// alone stages its own, a link where the source is on the temporary folder's volume, else one copy. The checks also set
// REE_SERVICE_LINK=1, so the services they start run from a link in their state dir instead of a copy of their own.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { realPath } from "../tools/release/common.mjs";
import { removeTestFolder } from "../tools/release/smoke.mjs";

const WINDOWS = process.platform === "win32";

// `dest` as a hard link to `source`, else a copy: across volumes, or where this user may not link the file (node under
// Program Files: making a link needs write access to the file)
export function linkOrCopy(source, dest) {
	try {
		fs.linkSync(source, dest);
	} catch (error) {
		if (error.code !== "EXDEV" && error.code !== "EPERM" && error.code !== "EACCES") throw error;
		fs.copyFileSync(source, dest);
	}
}

let stageDir = null;
const staged = new Map();

// The staged file `name` of `source`: the one in REE_TEST_STAGE, else this process's own
function stagedFile(name, source) {
	let file = staged.get(name);
	if (file) return file;
	file = process.env.REE_TEST_STAGE && path.join(process.env.REE_TEST_STAGE, name);
	if (!file || !fs.existsSync(file)) {
		stageDir ??= realPath(fs.mkdtempSync(path.join(os.tmpdir(), "ree-test-stage-")));
		file = path.join(stageDir, name);
		linkOrCopy(source, file);
		if (!WINDOWS) fs.chmodSync(file, 0o755);
	}
	staged.set(name, file);
	return file;
}

// The ree under test (REE_TEST_BINARY) installed at `dest`
export function installRee(dest) {
	linkOrCopy(stagedFile(WINDOWS ? "ree.exe" : "ree", process.env.REE_TEST_BINARY), dest);
	if (!WINDOWS) fs.chmodSync(dest, 0o755);
}

// This node installed at `dest` (Windows: an executable runs under its own name, so the worker is node itself)
export function installNode(dest) {
	linkOrCopy(stagedFile("node.exe", process.execPath), dest);
}

// Removes this process's own stage, if it made one (the caller's `after`)
export async function removeStage() {
	if (stageDir) await removeTestFolder(stageDir);
	stageDir = null;
	staged.clear();
}
