// Checks every internal link and asset reference in the built site: the target
// must exist under the base path, and a #fragment must match an id on the
// target page. starlight-links-validator can't do this here, because it maps
// pages from src/content/docs and the guide lives in ../docs/guide.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const site = 'https://jowch.github.io';
const base = '/Endeavor/';
const dist = fileURLToPath(new URL('../dist/', import.meta.url));

function htmlFiles(dir) {
	return readdirSync(dir).flatMap((name) => {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) return htmlFiles(path);
		return name.endsWith('.html') ? [path] : [];
	});
}

const idCache = new Map();
function ids(file) {
	if (!idCache.has(file)) {
		const html = readFileSync(file, 'utf8');
		idCache.set(file, new Set(['_top', ...[...html.matchAll(/\sid="([^"]*)"/g)].map((m) => m[1])]));
	}
	return idCache.get(file);
}

function targetFile(pathname) {
	const rel = decodeURIComponent(pathname.slice(base.length));
	const path = join(dist, rel);
	if (rel === '' || rel.endsWith('/')) return join(path, 'index.html');
	return existsSync(path) && statSync(path).isDirectory() ? join(path, 'index.html') : path;
}

const problems = [];
let checked = 0;
for (const file of htmlFiles(dist)) {
	const page = '/' + relative(dist, file).split('\\').join('/').replace(/index\.html$/, '');
	const pageUrl = new URL(base.slice(0, -1) + page, site);
	const html = readFileSync(file, 'utf8');
	for (const [tag] of html.matchAll(/<(?:a|link|script|img|source|meta)\b[^>]*>/g)) {
		// GitHub Pages serves 404.html for any missing path, so its canonical URL has no file.
		if (page === '/404.html' && tag.includes('rel="canonical"')) continue;
		const isImageMeta = /\s(?:property|name)="(?:og:image|twitter:image)"/.test(tag);
		for (const [, attr, raw] of tag.matchAll(/\s(href|src|srcset|content)="([^"]*)"/g)) {
			if (attr === 'content' && !isImageMeta) continue;
			const decoded = raw.replaceAll('&amp;', '&');
			const values = attr === 'srcset' ? decoded.split(',').map((c) => c.trim().split(/\s/)[0]) : [decoded.trim()];
			for (const value of values) {
				if (!value || /^(mailto|tel|data|javascript):/.test(value)) continue;
				const url = new URL(value, pageUrl);
				if (url.origin !== site) continue;
				checked++;
				const where = `${relative(dist, file)}: ${attr}="${value}"`;
				if (!url.pathname.startsWith(base)) {
					problems.push(`${where} is outside ${base}`);
					continue;
				}
				const target = targetFile(url.pathname);
				if (!existsSync(target)) {
					problems.push(`${where} points to a missing file`);
					continue;
				}
				const hash = decodeURIComponent(url.hash.slice(1));
				if (hash && target.endsWith('.html') && !ids(target).has(hash)) {
					problems.push(`${where} points to a missing #${hash}`);
				}
			}
		}
	}
}

if (problems.length) {
	console.error(`Broken links (${problems.length}):\n  ${problems.join('\n  ')}`);
	process.exit(1);
}
console.log(`Links OK: ${checked} internal links and assets checked.`);
