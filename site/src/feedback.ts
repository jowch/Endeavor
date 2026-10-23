// Where feedback goes: src/pages/feedback.astro lays it out. The fields both forms
// ask for are listed in site/feedback-fields.md.

// The Tally form's link, such as 'https://tally.so/r/abc123'. Until it is set, the
// feedback page offers GitHub only. The app and the plugin link to the page, not to
// Tally, so changing this needs no app release.
export const tallyForm = 'https://tally.so/r/gbDOYJ';

export const githubForm = 'https://github.com/jowch/Endeavor/issues/new?template=feedback.yml';

// The details the app (or a link) may pass on to either form. Anything else in the
// page's address is dropped.
export const fields = ['source', 'build', 'os', 'agent'] as const;
