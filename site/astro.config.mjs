import { defineConfig } from 'astro/config';
import { satteri } from '@astrojs/markdown-satteri';
import starlight from '@astrojs/starlight';
import { guideLinks } from './guide-links.mjs';

const base = '/Endeavor';
const guideDir = new URL('../docs/guide/', import.meta.url);

export default defineConfig({
	site: 'https://jowch.github.io',
	base,
	markdown: {
		processor: satteri({ hastPlugins: [guideLinks({ guideDir, base })] }),
	},
	vite: {
		server: { fs: { allow: ['..'] } },
	},
	integrations: [
		starlight({
			title: 'Endeavor',
			description: 'The user guide for Endeavor, a Mac app for data analysis with Claude in a Julia notebook.',
			logo: {
				light: '../assets/icon/logo-light.svg',
				dark: '../assets/icon/logo-dark.svg',
				replacesTitle: true,
			},
			favicon: '/favicon.svg',
			sidebar: [{ autogenerate: { directory: '../docs/guide' } }],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/jowch/Endeavor' }],
			editLink: { baseUrl: 'https://github.com/jowch/Endeavor/edit/main/site/' },
			markdown: { processedDirs: ['../docs/guide'] },
			customCss: [
				'@fontsource/schibsted-grotesk/400.css',
				'@fontsource/schibsted-grotesk/600.css',
				'@fontsource/schibsted-grotesk/700.css',
				'./src/styles/theme.css',
			],
		}),
	],
});
