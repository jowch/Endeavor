// /llms.txt: what an agent handed a link to this site needs, as plain text
// (https://llmstxt.org). The install text is site/agent-install.md, the same
// block the landing page, the install guide and both READMEs show.
import install from '../../agent-install.md?raw';

const site = 'https://jowch.github.io/Endeavor';

const body = install
	.replace(/<\/?details>\n?/g, '')
	.replace(/<summary>[\s\S]*?<\/summary>\n?/, '')
	.trim();

const text = `# Endeavor

> Endeavor is a desktop app for analyzing data with an AI agent, reproducibly. The agent (Claude Code, Codex or Antigravity) writes and runs code in a live Julia or R notebook while the user watches, edits and decides what runs. Python, through marimo, is coming soon. The notebook tools also work without the app, as the EndeavorMCP plugin for an agent the user already runs.

## Installing with an agent

${body}

## Docs

- [What Endeavor is](${site}/overview/): what it does and what it needs
- [Install Endeavor](${site}/install/): the desktop app for Mac and Windows, and building on Linux
- [EndeavorMCP README](https://github.com/jowch/EndeavorMCP#readme): the plugin, servers and clusters, updating and removing it
- [Troubleshooting](${site}/troubleshooting/)
`;

export function GET() {
	return new Response(text, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
}
