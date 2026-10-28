# Marketplace navigation

Status: Accepted

## Context

The desktop marketplace has one unfiltered skill list. Users need to browse by tag or repository. The web service already provides public tags, featured repositories, filtered skills, and skill lookup by id.

## Decision

- Use two columns with the existing split view and theme tokens.
- The left column has Categories and Hot repositories tabs. Each tab remembers its selection during the app session.
- Categories starts at All skills. Hot repositories includes only enabled entries with type `hot`, in API order. Its initial selection is the first entry. An empty list has no selection and no skill request.
- Only one tag or repository filter applies at a time.
- Search applies within the selection. A selection change clears search and resets pagination and scroll. Sort order is retained.
- Returning from skill details restores selection, search, sort, loaded results, and scroll position.
- Resolve remote details by marketplace id to distinguish skills with the same name.
- Use the public catalog endpoints for browsing. Keep authenticated install reporting and update lookup. Search telemetry for browsing uses the public channel.
- Ignore stale list responses. A failed next-page request must retry the same offset.
- Desktop is the first implementation. Change the backend if a required capability is missing. iOS is a later task.

## Consequences

The existing public tag endpoint loads all related skill summaries and timed out during verification. Add `GET /api/public/tags/summary` with the existing visible-skill count query. Keep the original endpoint unchanged. Prefer the summary endpoint. If an older server returns 404 or 422, use the original tags endpoint with a 45-second timeout because its response includes all related skills. Do not fall back for authorization, rate-limit, server, or decode errors. Deploy the summary endpoint to remove the large legacy response. Navigation state lasts for the app session, not across restarts.

## Verification

Test tab selection, scoped search, empty repositories, stale responses, failed pagination, and return state. Check Rust response mapping and request parameters. Run frontend checks, production build, workspace checks, and workspace tests. Inspect the layout in a browser with clearly identified fixture data when native execution is unavailable.
