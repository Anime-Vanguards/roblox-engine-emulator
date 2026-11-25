// What the resource suite measures, in plain words, and how each figure is checked against its baseline.
//
// Every metric has a one-sentence meaning a non-specialist can read (the job summary shows it beside the numbers), its
// technical name second, a unit, and a default tolerance. A gated metric fails the run when it goes over
// `baseline * ratio + slack`; report-only metrics (times, which swing on shared CI machines) are shown but never fail.
// The ratios stay under 2 so a doubling always fails; the slack keeps tiny figures (a few files, a few MB) from
// failing on noise. tools/resources/baseline.json can override any of these per metric.

export const METRICS = [
	{
		key: "ioWriteBytes", label: "Data written", technical: "bytes written (OS I/O accounting, the whole process tree)", unit: "bytes", gate: true, ratio: 1.5, slack: 8 << 20,
		meaning: "How much data this run saved to disk (log and pipe output included). Large amounts wear the SSD and make the whole PC stutter while it happens.",
	},
	{
		key: "diskGrowth", label: "Disk space kept", technical: "growth of the run's cache, state, output and temp folders", unit: "bytes", gate: true, ratio: 1.5, slack: 4 << 20,
		meaning: "How much more disk space the run's caches and outputs take up afterwards. It stays used until someone deletes it.",
	},
	{
		key: "filesLeft", label: "Leftover temporary files", technical: "files left in the run's temp folder, and *.tmp files in its caches", unit: "count", gate: true, ratio: 1.5, slack: 2,
		meaning: "Temporary files still on disk after the run ended. Nothing cleans them up, so they pile up run after run.",
	},
	{
		key: "filesCreated", label: "Files created", technical: "new files in the watched folders at the end of the run", unit: "count", gate: true, ratio: 1.5, slack: 16,
		meaning: "How many new files the run left in its caches and outputs. Many small files make disk scans, backups and deletes slow.",
	},
	{
		key: "peakWorkingSetMax", label: "Memory of the biggest process", technical: "peak working set of the largest single process", unit: "bytes", gate: true, ratio: 1.5, slack: 48 << 20,
		meaning: "The most RAM any one process held at once. When it is too high, other programs are pushed out of memory and the PC slows down.",
	},
	{
		key: "peakWorkingSetSum", label: "Memory of all processes together", technical: "peak of the summed working sets (sampled every 100 ms)", unit: "bytes", gate: true, ratio: 1.5, slack: 96 << 20,
		meaning: "The most RAM the whole run held at once, all its processes added up. This is what the rest of the PC has to make room for.",
	},
	{
		key: "peakPrivateSum", label: "Memory reserved", technical: "peak commit charge of the process tree (job peak memory used)", unit: "bytes", gate: true, ratio: 1.5, slack: 96 << 20,
		meaning: "The most memory the run claimed for itself at once, whether or not it was in RAM. Windows refuses new programs when every program's claims together run out.",
	},
	{
		key: "pageFaults", label: "Page faults", technical: "page faults, soft and hard (the whole process tree)", unit: "count", gate: true, ratio: 1.4, slack: 40000,
		meaning: "How many times a process touched memory that was not ready and had to wait for it, from RAM or from disk. Far more than usual means the run handles far more memory than it needs.",
	},
	{
		key: "processes", label: "Processes started", technical: "processes in the tree over the whole run", unit: "count", gate: true, ratio: 1.25, slack: 2,
		meaning: "How many programs the run started. Each one costs startup time and memory.",
	},
	{
		key: "leftRunning", label: "Processes left running", technical: "processes still alive after the command ended", unit: "count", gate: true, ratio: 1, slack: 0,
		meaning: "Programs the run started and never stopped. They keep holding memory until someone ends them.",
	},
	{
		key: "ioReadBytes", label: "Data read", technical: "bytes read (OS I/O accounting; cached reads count too)", unit: "bytes", gate: false, ratio: 1.8, slack: 32 << 20,
		meaning: "How much data the run read from files and pipes. Reading the same data over and over slows the run and the disk.",
	},
	{
		key: "peakDiskGrowth", label: "Most disk space used during the run", technical: "peak growth of the watched folders (sampled every second)", unit: "bytes", gate: false, ratio: 1.6, slack: 16 << 20,
		meaning: "The most extra disk space the run took at any moment, including files it deleted again before it ended.",
	},
	{
		key: "peakHandlesMax", label: "Open handles", technical: "peak handle count of the largest process", unit: "count", gate: false, ratio: 1.5, slack: 200,
		meaning: "The most files, threads and other system objects one process held open at once. A count that keeps climbing is a leak.",
	},
	{
		key: "peakProcesses", label: "Most processes at once", technical: "peak number of live processes (sampled)", unit: "count", gate: false, ratio: 1.5, slack: 2,
		meaning: "How many of the run's programs were running at the same moment.",
	},
	{
		key: "wallMs", label: "Time taken", technical: "wall-clock time", unit: "ms", gate: false, ratio: 1.5, slack: 2000,
		meaning: "How long the run took on the clock. Shown only: shared CI machines are faster or slower from run to run.",
	},
	{
		key: "cpuMs", label: "Processor time", technical: "CPU time, user plus kernel, all processes", unit: "ms", gate: false, ratio: 1.5, slack: 2000,
		meaning: "How much processor work the run did, all cores added up. Shown only, like the time taken.",
	},
	{
		key: "cpuKernelMs", label: "Processor time in the system", technical: "kernel CPU time", unit: "ms", gate: false, ratio: 1.5, slack: 1000,
		meaning: "The part of the processor work spent inside Windows or Linux itself, mostly on files and memory. It grows with heavy disk and memory use.",
	},
];

export const METRIC = Object.fromEntries(METRICS.map(metric => [metric.key, metric]));

// The tolerance for `key`: the defaults above, with what baseline.json sets for it on top
export function tolerance(key, overrides = {}) {
	const metric = METRIC[key];
	const own = overrides[key] || {};
	return {
		gate: own.gate ?? metric.gate,
		ratio: own.ratio ?? metric.ratio,
		slack: own.slack ?? metric.slack,
	};
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes) {
	if (bytes == null || Number.isNaN(bytes)) return "n/a";
	const sign = bytes < 0 ? "-" : "";
	let value = Math.abs(bytes), unit = 0;
	while (value >= 1000 && unit < UNITS.length - 1) {
		value /= 1024;
		unit++;
	}
	const digits = unit === 0 ? 0 : value >= 100 ? 0 : value >= 10 ? 1 : 2;
	return `${sign}${value.toFixed(digits)} ${UNITS[unit]}`;
}

export function formatCount(count) {
	if (count == null || Number.isNaN(count)) return "n/a";
	return Math.round(count).toLocaleString("en-US");
}

export function formatMs(ms) {
	if (ms == null || Number.isNaN(ms)) return "n/a";
	if (Math.abs(ms) < 1000) return `${Math.round(ms)} ms`;
	if (Math.abs(ms) < 120000) return `${(ms / 1000).toFixed(1)} s`;
	return `${(ms / 60000).toFixed(1)} min`;
}

export function format(key, value) {
	const unit = METRIC[key]?.unit;
	if (unit === "bytes") return formatBytes(value);
	if (unit === "ms") return formatMs(value);
	return formatCount(value);
}
