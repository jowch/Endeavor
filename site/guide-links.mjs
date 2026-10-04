import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// The guide links between pages as `./servers.md#anchor` so it reads on GitHub.
// On the site each page lives at `<base>/<id>/`, so rewrite those links to match.
export function guideLinks({ guideDir, base }) {
	const root = fileURLToPath(guideDir);
	return ({ fileURL }) => {
		if (!fileURL) return;
		const from = dirname(fileURLToPath(fileURL));
		if (relative(root, from).startsWith('..')) return;
		return {
			name: 'endeavor-guide-links',
			element: {
				filter: ['a'],
				visit(node, ctx) {
					const href = node.properties?.href;
					if (typeof href !== 'string') return;
					const match = /^([^:#?]+)\.md(#.*)?$/.exec(href);
					if (!match) return;
					const id = relative(root, resolve(from, match[1])).split(/[\\/]/).join('/').toLowerCase();
					ctx.setProperty(node, 'href', `${base}/${id}/${match[2] ?? ''}`);
				},
			},
		};
	};
}
