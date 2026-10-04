// All the words on the home page. Edit them here; src/pages/index.astro lays them out.
// `href` values starting with "/" are inside the site and get the /Endeavor base added.

export const meta = {
	title: 'Endeavor: data analysis with Claude in a Julia notebook',
	description:
		'Endeavor is a Mac app where Claude writes and runs Julia code in a live notebook beside your chat, while you watch, edit and decide what runs. Coming soon for macOS.',
	imageAlt: 'The Endeavor window, with the chat on the left and a Julia notebook with a plot on the right',
};

export const nav = {
	docs: { text: 'Docs', href: '/overview/' },
	github: { text: 'GitHub', href: 'https://github.com/jowch/Endeavor' },
};

export const hero = {
	name: 'Endeavor',
	tagline: 'Build our future',
	lead: 'A Mac app where Claude writes and runs Julia code in a live notebook beside your chat, while you watch, edit and decide what runs.',
	status: 'Coming soon for macOS',
	primary: { text: 'Read the guide', href: '/overview/' },
	secondary: { text: 'Use the notebook tools today', href: 'https://github.com/jowch/EndeavorMCP#readme' },
};

export const record = {
	heading: 'The notebook is the record',
	paragraphs: [
		'You describe what you want in the chat. Claude writes the code and runs it in a Pluto notebook next to it, and you see each step as it happens.',
		'The chat is where you talk. The notebook is what you keep. It holds every step of the analysis as code, with the result under it, so you or a colleague can read it, check it and run it again later.',
		'Endeavor is made for scientists who are taking on their own analysis and are new to code or notebooks. If you already write Julia, you can edit any cell yourself and give Claude more room.',
	],
	alt: 'The Endeavor window. On the left, the list of sessions. In the middle, the chat, with a request to simulate 1,000 coin flips and Claude’s reply. On the right, the notebook with three cells and a plot of the share of heads.',
	link: { text: 'What Endeavor is', href: '/overview/' },
};

export const features = [
	{
		id: 'approvals',
		heading: 'You decide what runs',
		paragraphs: [
			'Claude asks before it changes the notebook or runs code. A card above the message box shows the change and what else will run again, and you answer with one key.',
			'Pick a mode for how much Claude may do on its own. Manual asks before every change. Ask to run lets Claude edit and asks before running. Auto runs without asking. Plan only reads, then proposes a plan.',
		],
		image: 'approval-card',
		alt: 'An approval card above the message box asking “Edit fit and run it?”, with the change shown as a diff and the buttons Deny, Always this session, and Edit and run.',
		links: [{ text: 'Modes and approvals', href: '/modes-and-approvals/' }],
	},
	{
		id: 'safe-preview',
		heading: 'Open any notebook without running it',
		paragraphs: [
			'A notebook you open from disk starts in safe preview. You can read it and edit it, and none of its code runs until you click Run notebook.',
			'Code in a notebook can read, change or delete files. A notebook from a colleague, a download or an old project may do things you don’t expect, so you read it first.',
		],
		image: 'safe-preview',
		alt: 'A notebook in safe preview, with the Safe preview label in the header and a box at the top that says the file is open without running any code, with a Run notebook button.',
		links: [{ text: 'Safe preview', href: '/safe-preview/' }],
	},
	{
		id: 'point',
		heading: 'Ask about any part of the notebook',
		paragraphs: [
			'Turn on Point and click a cell, a plot or a paragraph, or drag a box over part of a plot. Then type your question about just that part.',
			'You can also select text in a cell, a result or a reply and press ⌘E to ask about it.',
		],
		image: 'point',
		alt: 'Point turned on: the notebook is dimmed, a plot is outlined and tagged Figure, and a bar under it holds the question “Why is the line not exactly at 0.5?”',
		links: [{ text: 'Point and Reply', href: '/sessions/#ask-about-one-part-of-the-notebook' }],
	},
	{
		id: 'where',
		heading: 'Run on this Mac, a server or a cluster',
		paragraphs: [
			'Each session runs its notebook where you choose: on this Mac, on a lab server over SSH, or as a job on a Slurm cluster. Claude stays on your Mac either way.',
			'On a cluster, pick a preset or set the partition, CPUs, memory and time limit, and Endeavor submits the job for you.',
		],
		image: 'cluster-resources',
		alt: 'The resources popover for a cluster session, with the Small, Medium and Large presets, the partition, and fields for CPUs, memory and time limit.',
		links: [{ text: 'Servers', href: '/servers/' }, { text: 'Clusters', href: '/clusters/' }],
	},
] as const;

export const withoutApp = {
	heading: 'Use the notebook tools without the app',
	paragraphs: [
		'If you already work with Claude Code, Codex or Gemini CLI, you can use Endeavor’s notebook tools today. They are a separate program, EndeavorMCP, that runs on your computer, a lab server or a cluster node.',
		'Run it in the folder for your notebooks. It prints a link to watch the notebook in your browser, and the settings to connect your agent over MCP.',
	],
	code: ['cargo install --git https://github.com/jowch/EndeavorMCP endeavor-mcp', 'cd ~/my-analysis', 'endeavor serve'],
	codeLabel: 'Install EndeavorMCP and start the notebook tools',
	link: { text: 'EndeavorMCP on GitHub', href: 'https://github.com/jowch/EndeavorMCP#readme' },
};

export const footer = {
	links: [
		{ text: 'Docs', href: '/overview/' },
		{ text: 'GitHub', href: 'https://github.com/jowch/Endeavor' },
		{ text: 'EndeavorMCP', href: 'https://github.com/jowch/EndeavorMCP' },
		{ text: 'MIT licence', href: 'https://github.com/jowch/Endeavor/blob/main/LICENSE' },
	],
};
