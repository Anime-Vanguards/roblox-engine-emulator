#!/usr/bin/env node
// The root process of a workload made of several commands, so the whole set is measured as one process tree:
//
//   node tools/resources/lib/fanout.mjs <plan.json>
//
// plan.json: { "steps": [ <command> | { "parallel": [<command>, ...] } ] }, run one after another; a parallel step
// starts its commands together and waits for all of them. A command is { "argv": [...], "cwd"?, "env"?: {...},
// "log": <file>, "stdin"?: <file fed to it>, "expect"?: <exit code, default 0> }. Exits 1 when a command ends with
// another code than it expects, after every step has run (so one failure does not hide what the others cost).

import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";

function run(command) {
	fs.mkdirSync(path.dirname(command.log), { recursive: true });
	const out = fs.openSync(command.log, "w");
	const input = command.stdin ? fs.openSync(command.stdin, "r") : "ignore";
	const child = spawn(command.argv[0], command.argv.slice(1), {
		cwd: command.cwd || process.cwd(),
		env: { ...process.env, ...(command.env || {}) },
		stdio: [input, out, out],
		windowsHide: true,
	});
	return new Promise(resolve => {
		child.on("error", error => {
			fs.writeSync(out, `fanout: ${error.message}\n`);
			fs.closeSync(out);
			resolve(false);
		});
		child.on("exit", code => {
			fs.closeSync(out);
			if (typeof input === "number") fs.closeSync(input);
			const ok = code === (command.expect ?? 0);
			if (!ok) process.stderr.write(`fanout: ${path.basename(command.argv[0])} exited ${code} (see ${command.log})\n`);
			resolve(ok);
		});
	});
}

const plan = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
let ok = true;
for (const step of plan.steps) {
	const results = await Promise.all((step.parallel || [step]).map(run));
	ok &&= results.every(Boolean);
}
process.exit(ok ? 0 : 1);
