# Issue tracker: GitHub Issues

Issues for this repo live in GitHub Issues at `Suyaj/Packsmith`, managed via the `gh` CLI.

## Conventions

- One issue per ticket; the ticket number is the GitHub issue number
- Triage state is recorded with labels (see `triage-labels.md` for the role → label mapping): `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`
- Blocking is expressed as a `**Blocked by:** #NN, #NN` line in the issue body; a ticket is unblocked when every listed issue is closed
- Discussion/comments live in the issue's native comment thread
- Historical tickets (01–26) were migrated from the local markdown archive and are closed with the `v1-archive` label; `.scratch/packsmith/` is kept as a read-only archive — do not create new tickets there

## When a skill says "publish to the issue tracker"

```bash
gh issue create --repo Suyaj/Packsmith --title "<NN>: <title>" --body-file <body.md> --label ready-for-agent
```

Include a `**Blocked by:**` line in the body for each gating issue (or omit when none).

## When a skill says "fetch the relevant ticket"

```bash
gh issue view <number> --repo Suyaj/Packsmith [--comments]
```

The user will normally pass the issue number directly.

## Wayfinding operations

Used by `/wayfinder`, adapted to GitHub:

- **Map**: `.scratch/<effort>/map.md` (unchanged; local file).
- **Child ticket**: a GitHub issue, title prefixed `NN:`; a `Type:` line in the body records the ticket type (`research`/`prototype`/`grilling`/`task`).
- **Blocking**: the `**Blocked by:** #NN, #NN` line in the body.
- **Frontier**: open issues whose blockers are all closed; first by number wins.
- **Claim**: assign yourself (`gh issue edit <n> --add-assignee @me`).
- **Resolve**: post the answer as an issue comment, then close.
