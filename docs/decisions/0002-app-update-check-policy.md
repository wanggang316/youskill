# App update check policy

Status: Accepted

## Context

The app does one silent app update check at startup. After that, it checks again only when the user selects "Check for Updates…" in the app menu or in Settings. A window that stays open for days does not find new releases.

## Decision

- Keep the silent check at startup.
- When the window gets focus, do a check only if 1 hour or more has passed since the last check.
- While the app runs, also do a check every hour. Focus alone is not sufficient because a window can stay focused or visible for a long time without a focus change.
- The last check time is the end of any check: automatic or manual, successful or failed. A manual check restarts the interval. When the app is offline, it does not retry on each focus.
- Automatic checks are silent. A new release shows only the Update button in the top bar. They do not open the update dialog.
- Continue the automatic checks after a release is found, so that the app can find a later release.
- Skip an automatic check while an install runs or while the update dialog is open. A check changes the dialog to its checking view.
- Keep the last check time in memory only. Each app start checks again.

## Consequences

Each automatic check requests `latest.json` from GitHub release assets. Release notes come from the unauthenticated GitHub API only when a release is found. That API is limited to 60 requests each hour for each IP address. After the network becomes available again, a release can stay unknown for up to 1 hour.

The Update button in the top bar starts an install without the dialog. That is a different topic and this decision does not change it.

## Reconsider when

- Users report that releases arrive too late, or that the request volume is too high.
- The updater endpoint moves off GitHub, or its rate limits change.
