// Tests of the downloads of tools/vendor.mjs (tools/download.mjs), against a local HTTP server:
// node --test tools/download.test.mjs (tools/luau-release.test.mjs runs them too)
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import { createHash } from "node:crypto";
import { test } from "node:test";
import { download } from "./download.mjs";

const ARCHIVE = Buffer.from("the pinned archive, as its release serves it");
const SHA256 = createHash("sha256").update(ARCHIVE).digest("hex");

// A server on a free local port that answers its requests in turn with `answers`, the last one from then on: a status
// (the archive with 200, an error page otherwise), "drop" (the connection closes before any answer), "cut" (it closes
// halfway through the archive) or "stall" (no answer at all). The server closes when the test ends.
async function serve(t, ...answers) {
	const requests = [];
	const server = http.createServer((request, response) => {
		const answer = answers[Math.min(requests.length, answers.length - 1)];
		requests.push(request.headers["user-agent"]);
		if (answer === "drop") {
			request.socket.destroy();
		} else if (answer === "cut") {
			response.writeHead(200, { "Content-Length": ARCHIVE.length });
			response.write(ARCHIVE.subarray(0, 8), () => request.socket.destroy());
		} else if (answer !== "stall") {
			response.writeHead(answer).end(answer === 200 ? ARCHIVE : "an error page");
		}
	});
	await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
	t.after(() => {
		server.closeAllConnections();
		server.close();
	});
	return { url: `http://127.0.0.1:${server.address().port}/lune-0.10.4.tar.gz`, requests };
}

// The pauses a download took (none is waited for) and what it reported, with the archive's pin
function options(more = {}) {
	const pauses = [];
	const logs = [];
	return { pauses, logs, sha256: SHA256, name: "lune-0.10.4.tar.gz", pause: async ms => void pauses.push(ms), log: message => logs.push(message), ...more };
}

test("an answer of 503 is tried again after 2 s, and the archive that comes next is used", async (t) => {
	const server = await serve(t, 503, 200);
	const o = options();
	assert.deepEqual(await download(server.url, o), ARCHIVE);
	assert.equal(server.requests.length, 2);
	assert.deepEqual(o.pauses, [2_000]);
	assert.equal(o.logs.length, 1);
	assert.match(o.logs[0], /lune-0\.10\.4\.tar\.gz: attempt 1 of 4 failed: HTTP 503; trying again in 2 s$/);
});

test("a connection that fails before the answer (fetch failed) is tried again, and the reason is reported", async (t) => {
	const server = await serve(t, "drop", 200);
	const o = options();
	assert.deepEqual(await download(server.url, o), ARCHIVE);
	assert.equal(server.requests.length, 2);
	assert.deepEqual(o.pauses, [2_000]);
	assert.match(o.logs[0], /attempt 1 of 4 failed: fetch failed: .+; trying again in 2 s$/);
});

test("a connection that closes halfway through the archive, an answer of 429 and one that stalls are tried again too", async (t) => {
	const cut = await serve(t, "cut", 200);
	const o = options();
	assert.deepEqual(await download(cut.url, o), ARCHIVE);
	assert.equal(cut.requests.length, 2);
	assert.match(o.logs[0], /attempt 1 of 4 failed: terminated/);

	const limited = await serve(t, 429, 200);
	const p = options();
	assert.deepEqual(await download(limited.url, p), ARCHIVE);
	assert.match(p.logs[0], /attempt 1 of 4 failed: HTTP 429/);

	const stalled = await serve(t, "stall", 200);
	const q = options({ timeout: 200 });
	assert.deepEqual(await download(stalled.url, q), ARCHIVE);
	assert.equal(stalled.requests.length, 2);
	assert.match(q.logs[0], /attempt 1 of 4 failed: no complete answer in 0\.2 s/);
});

test("the pauses double, 2, 4 and 8 s, and the fourth failure gives up with the address and its reason", async (t) => {
	const server = await serve(t, 502);
	const o = options();
	await assert.rejects(download(server.url, o), new RegExp(`^Error: ${server.url.replaceAll(".", "\\.")}: no download in 4 attempts, the last one failed: HTTP 502$`));
	assert.equal(server.requests.length, 4);
	assert.deepEqual(o.pauses, [2_000, 4_000, 8_000]);
	assert.equal(o.logs.length, 3);
});

test("an answer of 404 (or any other 4xx but 429) fails at once, with no second attempt", async (t) => {
	for (const status of [404, 403, 410]) {
		const server = await serve(t, status, 200);
		const o = options({ pause: () => assert.fail("it waited for another attempt") });
		await assert.rejects(download(server.url, o), { message: `${server.url}: HTTP ${status}` });
		assert.equal(server.requests.length, 1);
		assert.deepEqual(o.logs, []);
	}
});

test("an archive whose SHA-256 is not the pinned one fails at once, with no second attempt", async (t) => {
	const server = await serve(t, 200);
	const o = options({ sha256: "0".repeat(64), pause: () => assert.fail("it waited for another attempt") });
	await assert.rejects(download(server.url, o), { message: `lune-0.10.4.tar.gz: checksum mismatch, got ${SHA256}` });
	assert.equal(server.requests.length, 1);
	assert.deepEqual(o.logs, []);

	// a pin that names its hash but lost it is still checked, and fails; with no hash named the bytes are taken as they come
	await assert.rejects(download(server.url, options({ sha256: undefined })), /lune-0\.10\.4\.tar\.gz: checksum mismatch/);
	assert.deepEqual(await download(server.url), ARCHIVE);
});

test("tools/vendor.mjs downloads only through it, its pinned archives checked, and the headers it gives are sent", async (t) => {
	const vendor = fs.readFileSync(new URL("./vendor.mjs", import.meta.url), "utf8");
	assert.match(vendor, /^import \{ download as fetchBytes \} from "\.\/download\.mjs";$/m);
	assert.doesNotMatch(vendor, /\bfetch\(/);
	assert.match(vendor, /await download\(url, \{ sha256, name: file \}\)/);
	const server = await serve(t, 200);
	await download(server.url, { headers: { "User-Agent": "ree-vendor" } });
	assert.deepEqual(server.requests, ["ree-vendor"]);
});
