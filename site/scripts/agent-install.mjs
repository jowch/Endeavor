// Copies the "Installing with an agent?" block (site/agent-install.md) into
// the files that show it, between the agent-install markers. The landing page
// and llms.txt read the file itself when the site builds.
//
//   node scripts/agent-install.mjs            write the copies in this repo
//   node scripts/agent-install.mjs --check    fail if a copy differs (npm run build runs this)
//   node scripts/agent-install.mjs PATH...    also write PATH, such as ../../EndeavorMCP/README.md
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const repo = (p) => fileURLToPath(new URL('../../' + p, import.meta.url));
const source = 'site/agent-install.md';
const start = `<!-- agent-install: start. A copy of Endeavor's ${source}; edit it there and run site/scripts/agent-install.mjs. -->`;
const end = '<!-- agent-install: end -->';

const args = process.argv.slice(2);
const check = args.includes('--check');
const targets = [repo('README.md'), repo('docs/guide/install.md'), ...args.filter((a) => a !== '--check')];
const block = `${start}\n${readFileSync(repo(source), 'utf8').trim()}\n${end}`;

const problems = [];
for (const file of targets) {
	const text = readFileSync(file, 'utf8');
	const from = text.indexOf('<!-- agent-install: start');
	const to = text.indexOf(end);
	if (from < 0 || to < from) {
		problems.push(`${file}: no agent-install markers`);
		continue;
	}
	const next = text.slice(0, from) + block + text.slice(to + end.length);
	if (next === text) continue;
	if (check) problems.push(`${file} differs from ${source}: run node site/scripts/agent-install.mjs`);
	else {
		writeFileSync(file, next);
		console.log(`Wrote ${file}`);
	}
}
if (problems.length) {
	console.error(problems.join('\n'));
	process.exit(1);
}
