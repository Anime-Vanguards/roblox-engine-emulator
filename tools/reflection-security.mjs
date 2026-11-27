#!/usr/bin/env node
// Writes data/reflection-security.json from what tools/dump-reflection-security.luau returned in a playing client:
//
//   node tools/reflection-security.mjs <dump.json>
//
// It first checks that data/reflection.json is a game script's default view of the same Studio build (every class's members
// in the client's order, their permits), so the new file only holds what that view lacks:
//
// - `classes`: the classes data/reflection.json does not list, whole: those a Security filter shows (Player, CoreGui,
//   StudioService), the settings classes GetClasses never lists but the member queries answer (UserGameSettings,
//   GlobalSettings; `listed: false`), and a class named only as another's subclass (`subclass: true`).
// - `extend`: for a class of data/reflection.json, the properties a game script's view hides (`at`: the place among the
//   class's own properties when every one is shown) and the permits it hides (`permits`: an action and its capabilities, by
//   member).
// - the capabilities a class, its member queries and a property need beyond what their permits say: `list` where a class
//   has no New or GetService permit (an abstract one: those of the permit otherwise), `members` where the member queries
//   need fewer than GetClasses (Player), `visible` for a property with no Read permit (one with a Read permit needs that).
//
// It also writes each property's ContentType (`content`) into data/reflection.json, as text at the end of the property's
// object: every other byte stays.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DUMP = path.join(ROOT, "data", "reflection.json");
const OUT = path.join(ROOT, "data", "reflection-security.json");
// (the set's other names for two capabilities: SecurityCapabilities prints and holds them as these)
const ALIASES = { AssetRequire: "LoadUnownedAsset", Avatar: "AvatarAppearance" };

const [input, named] = process.argv.slice(2);
if (!input) {
	console.error("usage: node tools/reflection-security.mjs <the JSON tools/dump-reflection-security.luau returned> [<version>]");
	process.exit(2);
}
const raw = JSON.parse(fs.readFileSync(input, "utf8"));
const dumpText = fs.readFileSync(DUMP, "utf8");
const dump = JSON.parse(dumpText);
// (a playing client's version() is "?": name the build that ran it)
const version = /^\d+\.\d+/.test(raw.version) ? raw.version : named;
if (version !== dump.version) {
	console.error(`reflection-security: the capture is ${version ?? `${raw.version} (name its version)`}, data/reflection.json ${dump.version}: refresh them together`);
	process.exit(1);
}
raw.version = version;
const empty = value => value == null || (Array.isArray(value) && value.length === 0) ? {} : value;
// (capabilities as SecurityCapabilities prints them: in src/lua/prelude.luau CapabilityOrder)
const ORDER = fs.readFileSync(path.join(ROOT, "src", "lua", "prelude.luau"), "utf8").match(/local CapabilityOrder = {([^}]*)}/)[1].match(/"([A-Za-z]+)"/g).map(name => name.slice(1, -1));
const rank = name => { const at = ORDER.indexOf(name); return at < 0 ? ORDER.length : at; };
const capabilities = list => [...new Set((list ?? []).map(name => ALIASES[name] ?? name))].sort((a, b) => rank(a) - rank(b) || (a < b ? -1 : 1)).join(" | ");
const D = Object.fromEntries(dump.classes.map(c => [c.name, c]));
const problems = [];

// data/reflection.json must be the default view: the same members in the same order, with the same permits
for (const c of dump.classes) {
	const members = raw.members[c.name];
	if (!members) { problems.push(`${c.name}: the client's member queries do not answer it`); continue; }
	for (const kind of ["properties", "methods", "events"]) {
		const shown = members[kind].filter(m => members.default[kind][m.name] !== undefined).map(m => m.name);
		const ours = c[kind].map(m => m.name);
		if (shown.join() !== ours.join()) problems.push(`${c.name}.${kind}: data/reflection.json ${ours.join(",")} | client ${shown.join(",")}`);
		for (const m of c[kind]) {
			const theirs = members.default[kind][m.name];
			if (theirs && JSON.stringify(Object.entries(empty(m.permits)).sort()) !== JSON.stringify(Object.entries(empty(theirs)).sort())) problems.push(`${c.name}.${m.name}: permits differ`);
		}
	}
}
if (problems.length) {
	console.error(`reflection-security: data/reflection.json is not a game script's view of ${raw.version}:\n${problems.slice(0, 20).join("\n")}`);
	process.exit(1);
}

const gate = (table, key) => capabilities(raw.gates[table][key]);
const permitText = permits => Object.fromEntries(Object.entries(empty(permits)).sort(([a], [b]) => a < b ? -1 : 1));
function property(c, p) {
	const entry = { name: p.name, type: p.type, serialized: p.serialized };
	if (p.content) entry.content = p.content;
	entry.permits = permitText(p.permits);
	entry.display = empty(p.display);
	if (entry.permits.Read === undefined && gate("visible", `${c}.${p.name}`)) entry.visible = gate("visible", `${c}.${p.name}`);
	return entry;
}
const method = m => ({ name: m.name, returns: m.returns, parameters: m.parameters, permits: permitText(m.permits), display: empty(m.display) });
const event = e => ({ name: e.name, parameters: e.parameters, permits: permitText(e.permits), display: empty(e.display) });

const listed = Object.fromEntries(raw.classes.map(c => [c.name, c]));
const classes = [];
const extend = [];
for (const name of Object.keys(raw.superclasses).sort()) {
	const members = raw.members[name];
	const c = listed[name];
	if (D[name]) {
		const own = D[name];
		const entry = { name };
		const permit = own.permits?.New ?? own.permits?.GetService;
		if (permit === undefined && gate("list", name)) entry.list = gate("list", name);
		if (gate("members", name) !== (permit === undefined ? gate("list", name) : capabilities((permit || "").split(" | ").filter(Boolean)))) problems.push(`${name}: member queries need ${gate("members", name)}`);
		const hidden = [];
		members.properties.forEach((p, at) => { if (members.default.properties[p.name] === undefined) hidden.push({ at, ...property(name, p) }); });
		if (hidden.length) entry.properties = hidden;
		const permits = {};
		for (const kind of ["properties", "methods", "events"]) {
			for (const m of members[kind]) {
				const shown = members.default[kind][m.name];
				if (shown === undefined) continue;
				const more = Object.fromEntries(Object.entries(empty(m.permits)).filter(([action]) => shown[action] === undefined));
				if (Object.keys(more).length) (permits[kind] ??= {})[m.name] = permitText(more);
			}
		}
		if (Object.keys(permits).length) entry.permits = permits;
		const visible = {};
		for (const p of own.properties) if (empty(p.permits).Read === undefined && gate("visible", `${name}.${p.name}`)) visible[p.name] = gate("visible", `${name}.${p.name}`);
		if (Object.keys(visible).length) entry.visible = visible;
		if (Object.keys(entry).length > 1) extend.push(entry);
		continue;
	}
	const entry = { name, superclass: raw.superclasses[name] ?? undefined };
	if (!c && !members) { entry.subclass = true; classes.push(entry); continue; }
	if (c) {
		if (c.serialized) entry.serialized = true;
		entry.display = empty(c.display);
		entry.permits = permitText(c.permits);
		const permit = c.permits.New ?? c.permits.GetService;
		const list = permit === undefined ? gate("list", name) : capabilities(permit.split(" | ").filter(Boolean));
		if (permit === undefined && list) entry.list = list;
		if (gate("members", name) !== list) entry.members = gate("members", name);
	} else {
		entry.listed = false;
		if (gate("members", name)) entry.members = gate("members", name);
	}
	entry.properties = members.properties.map(p => property(name, p));
	entry.methods = members.methods.map(method);
	entry.events = members.events.map(event);
	classes.push(entry);
}
if (problems.length) {
	console.error(`reflection-security: ${problems.slice(0, 20).join("\n")}`);
	process.exit(1);
}
const lines = list => list.map(item => `\t\t${JSON.stringify(item)}`).join(",\n");
fs.writeFileSync(OUT, `{\n\t"version": ${JSON.stringify(raw.version)},\n\t"classes": [\n${lines(classes)}\n\t],\n\t"extend": [\n${lines(extend)}\n\t]\n}\n`);
console.log(`reflection-security: ${classes.length} classes, ${extend.length} extended, ${fs.statSync(OUT).size} bytes`);

// ContentType into data/reflection.json: a `"content":"<AssetType>"` field after each property's name where the client gives one
let text = dumpText;
const inserts = [];
let added = 0, kept = 0;
// (the offsets of each class's property objects, from a scan of the text: data/reflection.json is one line, strings never hold
// an unescaped quote)
const scan = locateProperties(text);
for (const c of dump.classes) {
	const members = raw.members[c.name];
	c.properties.forEach((p, index) => {
		const theirs = members.properties.find(m => m.name === p.name);
		if (!theirs?.content) return;
		if (p.content === theirs.content) { kept++; return; }
		if (p.content !== undefined) { problems.push(`${c.name}.${p.name}: content ${p.content} | client ${theirs.content}`); return; }
		const at = scan.get(`${c.name}\u0000${index}`);
		inserts.push({ at, text: `,"content":${JSON.stringify(theirs.content)}` });
		added++;
	});
}
if (problems.length) {
	console.error(`reflection-security: ${problems.join("\n")}`);
	process.exit(1);
}
for (const insert of inserts.sort((a, b) => b.at - a.at)) text = text.slice(0, insert.at) + insert.text + text.slice(insert.at);
if (added) fs.writeFileSync(DUMP, text);
console.log(`reflection-security: content written for ${added} properties of data/reflection.json (${kept} had it)`);

// For every class object of `classes`, the offset of each of its properties' closing brace.
function locateProperties(source) {
	let i = 0;
	const out = new Map();
	const ws = () => { while (/\s/.test(source[i])) i++; };
	function string() {
		const start = ++i;
		while (source[i] !== '"') i += source[i] === "\\" ? 2 : 1;
		return JSON.parse(source.slice(start - 1, ++i));
	}
	function value(pathKeys) {
		ws();
		const ch = source[i];
		if (ch === "{") {
			i++;
			const fields = {};
			ws();
			if (source[i] === "}") { i++; return { fields, end: i - 1 }; }
			for (;;) {
				ws();
				const key = string();
				ws(); i++; // :
				fields[key] = value([...pathKeys, key]);
				ws();
				if (source[i] === ",") { i++; continue; }
				i++; // }
				return { fields, end: i - 1 };
			}
		}
		if (ch === "[") {
			i++;
			const items = [];
			ws();
			if (source[i] === "]") { i++; return { items }; }
			for (;;) {
				items.push(value([...pathKeys, items.length]));
				ws();
				if (source[i] === ",") { i++; continue; }
				i++;
				return { items };
			}
		}
		if (ch === '"') return { string: string() };
		const start = i;
		while (!/[,\]}\s]/.test(source[i])) i++;
		return { scalar: source.slice(start, i) };
	}
	const root = value([]);
	for (const c of root.fields.classes.items) {
		const name = c.fields.name.string;
		(c.fields.properties?.items ?? []).forEach((p, index) => out.set(`${name}\u0000${index}`, p.end));
	}
	return out;
}
