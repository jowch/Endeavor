# Feedback fields

The feedback form on the site (Tally, linked from `/feedback/`) and the GitHub
issue form `.github/ISSUE_TEMPLATE/feedback.yml` (here and in EndeavorMCP) ask
the same things, so the daily triage reads them the same way. Change them
together.

| Field | Kind | Tally | GitHub form id | Filled by |
|---|---|---|---|---|
| What happened, or what got in your way? | long text, required | visible | `what` | the person |
| What were you trying to do? | short text | visible | `trying` | the person |
| Email, if you'd like a reply | email, optional; used only to reply | visible | none (GitHub has the account) | the person |
| Build, such as `0.1.0-6ed0c03` | short text | hidden | `build` | the app |
| OS: `macos`, `windows`, `linux` | short text | hidden | `os` | the app |
| Agent: `claude`, `codex`, `antigravity`, `plugin` | short text | hidden | `agent` | the app, or the plugin skill (`plugin`) |
| From: `app`, `site`, `plugin` | short text | hidden | `source` | the link |

The app fills build, os, agent and source in the link it opens
(`src/about.rs`, `feedback_url` and `github_feedback_url`). The site's
`/feedback/` page passes them on to both forms. No paths, hosts or log lines go
in a link.
