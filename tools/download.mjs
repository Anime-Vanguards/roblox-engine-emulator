// The downloads of tools/vendor.mjs (tests: tools/download.test.mjs). Every CI and release run downloads the vendored
// VM's pinned archives, so one answer that fails on the way must not fail the run. An attempt that fails on the network
// (fetch failed, a connection reset or closed halfway through the bytes, no complete answer within `timeout` ms) or on
// the server (HTTP 5xx, or 429: too many requests) is tried again, `attempts` times in all, after a pause that doubles
// each time: 2, 4 and 8 s by default. Any other answer that is no success (404, 403, ...) fails at once, and so do bytes
// whose SHA-256 is not the pinned one: integrity is never tried again.
import { createHash } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";

const digest = bytes => createHash("sha256").update(bytes).digest("hex");

// An answer that is no success: `transient` when another attempt may get another answer
class HttpError extends Error {
	constructor(status) {
		super(`HTTP ${status}`);
		this.transient = status >= 500 || status === 429;
	}
}

// Why an attempt failed. Of a network error fetch says only "fetch failed" (or "terminated" while reading the bytes) and
// keeps the reason in `cause`
function reason(error, timeout) {
	if (error.name === "TimeoutError") return `no complete answer in ${timeout / 1000} s`;
	const cause = error.cause;
	if (!cause) return error.message;
	const detail = cause.message && cause.code && !cause.message.includes(cause.code) ? `${cause.message} (${cause.code})` : cause.message || cause.code;
	return detail ? `${error.message}: ${detail}` : error.message;
}

// The bytes at `url`, checked against `sha256` when the options name one (`name` names the bytes in that error; a
// `sha256` that is there but undefined fails the check too, so a pin that lost its hash is never taken unchecked).
// `get`, `pause` and `log` stand in for fetch, the pause between attempts and where a retry is reported.
export async function download(url, options = {}) {
	const { sha256, name = url, headers, attempts = 4, wait = 2_000, timeout = 120_000, get = fetch, pause = sleep, log = console.error } = options;
	const pinned = Object.hasOwn(options, "sha256");
	for (let attempt = 1; ; attempt++) {
		let bytes;
		try {
			const response = await get(url, { headers, signal: AbortSignal.timeout(timeout) });
			if (!response.ok) {
				// (the error page is not read: drop it, freeing the connection)
				await response.body?.cancel().catch(() => {});
				throw new HttpError(response.status);
			}
			bytes = Buffer.from(await response.arrayBuffer());
		} catch (error) {
			// anything else that fails here failed on the network: fetch rejects with it, or reading the bytes does
			if (error instanceof HttpError && !error.transient) throw new Error(`${url}: ${error.message}`);
			if (attempt >= attempts) throw new Error(`${url}: no download in ${attempts} attempts, the last one failed: ${reason(error, timeout)}`);
			const delay = wait * 2 ** (attempt - 1);
			log(`${url}: attempt ${attempt} of ${attempts} failed: ${reason(error, timeout)}; trying again in ${delay / 1000} s`);
			await pause(delay);
			continue;
		}
		if (pinned) {
			const actual = digest(bytes);
			if (actual !== sha256) throw new Error(`${name}: checksum mismatch, got ${actual}`);
		}
		return bytes;
	}
}
