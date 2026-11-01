# Architecture

YouSkill is a centralized skill manager: every managed skill lives once in a local hub,
and every agent app (user-level or per project) receives a copy or a symlink of that hub
entry. This document is the map of the storage layout, the lock file, and the drift model.
Code is the source of truth; when this document and the code disagree, fix the document.

## Storage layout

```
~/.youskill/
  skills/<id>/<name>/     # hub: the canonical copy of each managed skill
  .skill-lock.json        # registry of sources, hashes and install targets
  instructions/<id>.md    # instruction templates: one Markdown file per template
  .instruction-lock.json  # registry of template names, hashes and install targets
  .trash/<name>-<stamp>/  # previous hub copy before a replacement (swept after 7 days)
  .sync-state.json       # library sync: repository, device id, last synced commit and its files (local only)
  .sync-staging/          # pulled skills assembled before they move into the hub
  .sync-checkpoint/       # copies taken before the last restore, for undo

<config_dir>/youskill/    # app config, custom agent apps, registered projects (unchanged)
  favorites.json          # local favorites of marketplace skills, cursor and synced account
```

Agent apps are defined in `src-tauri/crates/youskill-core/src/services/agent_apps_service.rs`. Each app has a
user-level skills directory and a project-level one. Apps that natively read the shared
directories (`~/.agents/skills` and `.agents/skills`: Codex, Cursor, GitHub Copilot, VS
Code, Gemini CLI, OpenCode, Kimi, Warp, Zed) use them as their paths, so one install
serves every such app; `detect_path` (e.g. `~/.cursor`) decides whether the app is shown as
installed. Apps with their own directories (Claude Code, Cline, Windsurf, ...) keep them.
`LEGACY_USER_ROOTS` lists the app-specific directories those apps used before adopting the
shared one; migration and scanning still recognise skills there as targets of that app.
The built-in `agents` app is the shared directory itself as an explicit target. Each app
also carries the instruction files it reads: `profile_path` inside a project (`CLAUDE.md`
for Claude Code, `AGENTS.md` elsewhere) and `global_profile_path` at user level
(`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`). `list_memory_files` resolves them for one
scope and groups the agents that read the same file; a
symlinked file (`CLAUDE.md -> AGENTS.md`) stays its own entry rather than merging into its
target.

## Workspaces and projects

A workspace is a folder that holds projects, stored in `<config_dir>/youskill/user_workspaces.json`.
Scanning one walks up to four levels deep and reports a folder as a project when it has an
agent's project skills directory (`project_path`) or an agent instruction file
(`profile_path`); the walk stops at each project and skips hidden and excluded folders.
Registered projects (`user_projects.json`) keep the `workspacePath` they were found in, so
the projects page can group them; projects registered before workspaces existed have none.

## Lock file (`~/.youskill/.skill-lock.json`)

```json
{
  "version": 2,
  "migration": { "completedAt": "...", "imported": ["..."], "installs": 3, "errors": [] },
  "skills": {
    "<id>": {
      "name": "foo",
      "source": { "type": "github", "repo": "owner/repo", "url": "...", "skillPath": "skills/foo/SKILL.md",
                  "branch": "main", "remoteSha": "...", "latestRemoteSha": "...", "checkedAt": "..." },
      "hash": "<sha256 of the hub directory at import / last accepted change>",
      "importedAt": "...",
      "updatedAt": "...",
      "installs": [
        { "scope": "user" | "project", "projectPath": "/abs" | null,
          "path": "/abs/target/dir", "mode": "copy" | "symlink",
          "hash": "<hub hash the target was written from>", "installedAt": "...",
          "agentIds": ["codex", "cursor"] }
      ]
    }
  }
}
```

- The record key is a UUID; it never changes. `name` (SKILL.md frontmatter `name`,
  falling back to the directory name) is the install directory name and the name agents
  use. Names are validated as safe directory names and compared case-insensitively, and
  more than one skill can have a name. The hub copy is `skills/<id>/<name>`, so the
  directory that holds SKILL.md keeps the skill name also behind a resolved symlink. The
  design is in [SKILL_IDENTITY.md](SKILL_IDENTITY.md).
- `source` is one of `github`, `folder`, `zip`, `none`. `folder` is only used for
  directories outside every agent root; a folder inside an agent root is registered as an
  install instead. The source key (`SkillSource::key`) is
  `github:<owner>/<repo>@<branch>:<skillDir>`, `folder:<path>`, or only the type for `zip`
  and `none`. A GitHub or folder source is in the hub at most once: importing it again
  replaces its skill. Any other import adds a new skill unless it names one in
  `replaceId`; `plan_import` reports the matches and the default (the newest same-name zip
  skill for a zip import).
- Commands take an id, a source key (`owner/repo[@branch]:<dir or name>`) or a name that
  one skill has (`LockFile::resolve`, `hub_service::resolve_skill`).
- One install directory belongs to one skill. An install onto a directory that another
  same-name skill has returns an `occupied by` blocker; forced, the install record moves
  and the old copy goes to `.trash`.
- Install records are keyed by target path. Several agent apps that share a directory
  (every agent using `.agents/skills` inside a project) share one record via `agentIds`.
  Uninstalling removes an agent id; files are deleted only when no agent is left.

## Content hash

`src-tauri/crates/youskill-core/src/utils/hash.rs::hash_dir` is the only content hash: sha256 over entries
sorted by relative path (`/` separators), excluding `.git`, `node_modules`, `.DS_Store`,
`Thumbs.db`. Files contribute `rel 0x00 content 0xFF`, symlinks `rel 0x01 target 0xFF`.
A stat-only signature caches results in memory so listing stays cheap.

## Drift model

Three nodes per skill: source S, hub H, targets T. `list_hub_skills` computes everything
offline; `check_source_updates` is the only network call and stores its result in the lock.
It batches: the marketplace answers for the skills it indexes, then one GitHub trees request
per repository and branch covers the rest. The app runs it for all skills at startup and when
the library is opened from the sidebar (at most every five minutes), and on every library
refresh (`checkAllSourceUpdates` in `stores/hub.ts`).

| Node             | States                                                                                           | Rule                                                                                                                              |
| ---------------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| Hub              | `ok`, `modified`, `missing`, `invalid`, `name_mismatch`                                          | `hash(H) != record.hash` is `modified`; a frontmatter name other than `record.name` is `name_mismatch`                            |
| Target (copy)    | `in_sync`, `outdated`, `modified`, `conflict`, `missing`                                         | three-way against `install.hash` as base: T==H in sync; T==base, H!=base outdated; T!=base, H==base modified; all differ conflict |
| Target (symlink) | `in_sync`, `broken_link`                                                                         | link into the hub is always in sync                                                                                               |
| Source           | `in_sync`, `update_available`, `source_modified`, `source_missing`, `not_checkable`, `unchecked` | github compares `remoteSha` with `latestRemoteSha`; folder hashes the folder                                                      |

Sync actions (`sync_skill`): `pull_source`, `push_targets`, `adopt_target`, `accept_hub`,
`push_source`. Any action that would discard local edits returns `applied: false` with
`blockers`; the caller may retry with `force`. Hub replacements move the old copy to
`.trash` first.

## Instructions: templates and agent files

Agent instruction files (`AGENTS.md`, `CLAUDE.md`, ...) are handled in two layers. The
**agent files** themselves, at user level (`global_profile_path`) and in every registered
project (`<project>/<profile_path>`), are listed by `list_instruction_files` and read and
written in place (`read_instruction_file`, `write_instruction_file`, which only accept
paths that are agent locations); nothing is copied anywhere for that. **Templates** are the
library: `~/.youskill/instructions/<id>.md`, keyed by a generated uuid, with a free-text
display name that need not be unique. `.instruction-lock.json` holds
`{ "instructions": { "<id>": { "name", "source", "hash", "importedAt", "updatedAt", "installs": [...] } } }`
with the same install records as skills (an `InstallRequest.name` carries the id). An
install target is the file an agent reads in a scope, so agents that read the same file
share one record. Because there is one file per agent location, installing where another
template is already installed is refused unless forced, which moves the install record
over; promoting an agent file to a template (`import_instructions` with the file's own
path) registers it as an install and takes the path over from any other template. Drift
is the same three-way comparison (`compare_three_way`) on file hashes with push, adopt and
accept, and `list_instruction_files` reports each file's template and state. The `source`
(`github` with repo, file path and branch; `file`; `agent` for a promoted agent file;
`none` when created in the app) is shown, nothing syncs with it. Imports come from picked
files or a folder (`detect_instruction_files`, Markdown files up to six levels deep,
skipping VCS, dependency and most hidden folders) or a GitHub repository, folder or file
URL (`detect_instruction_github`, downloaded to a temp directory). `write_instruction`
accepts new content as the template version and pushes it to copy targets that were in
sync; writing an agent file that is a symlink into the library does the same. Records
from before ids (keyed by a generated name, with every agent file copied in) are migrated
on the first listing: a record that only mirrored one agent file is dropped again, the
rest get an id and keep their key as name. Everything lives in `instruction_service`.

## Library sync (`library_sync_service`)

The library is backed up to a private GitHub repository through the GitHub REST API (Git
Data API, no local clone), with the token of the account selected in Settings > GitHub
(`gh`). The repository (layout version 2, [ADR 0004](decisions/0004-library-sync-item-scope.md))
holds `library.json` (per skill and instruction: name, source without update-check fields,
hash, timestamps and `scope`; a local source, skill `folder` / `zip` and instruction `file` /
`agent`, keeps its kind with an empty `path`), `agents.json` (custom agents, each with `scope`),
`skills/<id>/<name>/`, `instructions/<id>.md` and one device file per computer,
`devices/<device-id>.json` (device name, user-level installs by agent id as item kind and
id, time of the last backup). `scope` is `"shared"` or `{ "device": "<device-id>" }`; every
item is shared for now. No absolute path is written. A restore keeps the local path of an
item whose local source has the same kind; a new item gets the kind without a path. A
`library.json` written with paths is read, and the next backup writes it without them. The lock files stay the source of
truth: a backup exports a snapshot and commits it on top of the branch, a restore merges the
branch, or an earlier commit, back into the hub and lock files.

A device id is a random uuid made on the first connect, or on the first sync of a state
written before version 2, and kept in the sync state. `device_gets` is the one function that
selects the items of a device: shared items and its own device-only items. Snapshot, plan,
restore and pending see only those; a backup and the commit of a restore write the full
tree, with the device-only items and device files of other devices copied from the head.

`.sync-state.json` records the device id, the last synced commit (the base), its files,
`library.json`, custom agents and this device's file, the user-level agents installed here
at that point, the deferred items and the incoming items. A snapshot's device file is the
base device file plus this computer's install changes since the base (without a device file
at the base, the local installs). `backup` commits on top of the base when the head is the
base. When another device moved the head, it compares each item of this device in base,
snapshot and head with the restore rules: without a conflict it commits on top of the head
and moves the base there. Head-only changes and removals are not written here; they become
incoming items, which snapshots carry like deferred items (so later backups do not revert
them) and which the next plan offers compared with their old base, selected by default.
With a conflict the backup returns `diverged` (`remote_ahead` without local changes) and
writes nothing; on a first connect to a repository with a library it returns
`remote_not_empty`; the user then restores.

A repository with a version 1 `library.json` (with `agents`, no scopes or device files) is
migrated once: `backup` returns `migration_required` and `plan_restore` the plan state
`migration_required`, the UI asks for confirmation (other computers must update the app),
and `migrate` writes one commit with version 2, every item shared, no `agents`, and this
device's file from its local install records (on a first connect, local items with another
id are paired with repository items as the merge pairs them). After a sync the migration commit
becomes the base, as after a backup on a moved head: changes another device made since the
base become incoming items; with an item changed on both sides the base stays. A computer whose base is a version 1 commit
reads it as version 2 with every item shared and no installs, reads its custom agents from
the repository, and adds its device file with its next backup. Releases up to 0.10.0 refuse a
`library.json` above version 1. `status` reports `legacyBase` while the base is a version 1
commit, so Settings says that the next backup or restore may ask for the migration.

Restore is two calls. `plan_restore(mode, target)` compares each item of this device (a hub
skill by id, an instruction template by id, a custom agent by id) in three states: base, local snapshot and the
target commit (branch head, or a commit from `history`). Equality is the library entry plus
the item's files. Changed on one
side only takes that side (`keep_local`, `take_remote`); changed on both sides to the same
files, name and source takes the repository's hash and timestamps; otherwise it is a
`conflict`. A remote removal of an
item unchanged here is a `removal_candidate`. Modes: `pull`; on a first connect (no base,
both sides have a library, `pull` returns `choose_mode`) `merge`, `replace_local` (local is
the base, so the repository wins) and `replace_remote` (the repository is the base, so
local wins). `merge` and `replace_local` first pair local items with repository items of
another id: equal files is the same item, an equal GitHub source key with other files is
a conflict (a folder path is local to one computer and does not pair items) ([ADR 0003](decisions/0003-library-sync-id-on-first-merge.md)); custom
agents pair only by id. `replace_remote` replaces the shared items and keeps the parts of
other devices. The
plan carries the head and a fingerprint of the local snapshot.

`apply_restore(plan, decisions, options)` holds the operation lock from the check to the
commit. It plans again and returns `stale` without writing when the head or the
fingerprint changed. It then downloads the chosen repository items and assembles them in
the staging directory; a download failure or a missing or invalid item stops the restore
before anything local changes. Then it takes the checkpoint, rekeys paired items (hub
directory or template file, lock record, symlink installs), adds the local copy of a
`keep_both` item under a new id and a suffixed name (a paired item is rekeyed first, so the
id and its installs get the repository version; the copy loses its source when it is the
repository item's), writes the staged items (symlink installs follow a renamed hub
directory) and runs the confirmed removals through the normal hub and instruction removal.
Only when all of these succeed is the base set to the target, with the target's device file
of this device as the base device file, so the pending list shows what it changes. A
target without that file (from before this device joined, a head before its first backup, a
version 1 commit) keeps the installs as they are; on a failure the base stays
and the checkpoint undoes what was written. When the result differs from the head (local
items, kept removals, `keep_local`, a historical target), it is committed on top of the
head and the branch fast-forwards; a rejected fast-forward returns `stale` with the local
result kept. With `applyInstalls`, the install targets of the pending installs and copy
updates are added to the checkpoint, then the installs run.

Excluded `take_remote` items are deferred: the sync state keeps their repository version,
local snapshots carry that version instead of the local one (so they are neither a local
change nor removed by a backup), and the next plan offers them again with `deferred: true`
(excluded by default) until the repository changes them. The checkpoint copies both lock
files, the sync state, `user_agent_apps.json`, every hub id directory or template file the
restore touches and the install targets it writes, plus the symlink installs into them,
with a fingerprint of each path before and after the restore. `undo_restore` puts them back
and does not touch the repository. It refuses when a path changed after the restore
(the skill lock is compared without the source check fields), and it keeps the checkpoint
until every step succeeded, so a failed undo can be retried. One checkpoint is kept; a new
one replaces it only when the next restore is about to write.

`pending` compares this device file with the local installs and lists user-level installs,
uninstalls and copy updates; installs in other device files never show. It is usually empty
and fills after a restore of a commit with another device file of this device.
`apply_pending` runs the selected ones through the normal install actions without force.
Project installs are not synced.

## Favorites (`favorites_service`, `favorite_sync_service`)

A favorite is a marketplace skill, keyed by its marketplace id. `favorites.json` holds one
record per skill: favorite state, `updatedAt` (client milliseconds), a snapshot (name,
source, description) and a `pending` flag for changes not sent to an account. A removal
stays as a pending tombstone until a sync confirms it. Favorites work without an account;
the Marketplace has a Favorites view and a heart toggle in the list and the detail page. A
favorite whose skill lookup returns 404 shows as unavailable from its snapshot.

`favorite_sync_service::sync` pushes pending records, then pulls from the stored cursor
until the server has no more pages. A server record replaces the local one unless a newer
local change is still pending (last write wins on `updatedAt`). A rejected add (limit) is
reverted and reported. `Unauthorized` leaves the records as they are. `begin_account`
marks all records pending and clears the cursor when the account changes, so the first
sign-in merges local favorites as a union. The engine talks to the server through the
`FavoriteServer` trait; the HTTP client, sign-in and token storage wait for the server API
(youskill-web#68).

## Global Search

`search_service::search(env, query, user_name)` searches hub skills (name, description,
`SKILL.md` body without frontmatter), instruction templates (display name, body) and
install scopes (the user scope and every registered project, by name and path). It finds
each `SKILL.md` from the lock record (`skills/<id>/<name>`) and keys skill hits by id.
Names match as a case-insensitive fuzzy subsequence, the other fields as a case-insensitive
substring. Rank: exact name, name prefix, fuzzy name, description, path, body; equal
ranks keep their order. Groups come as skills, instructions, projects, with at most eight
hits each; empty groups are left out. A content hit carries a snippet of about 80
characters; positions count characters, not bytes. Files are read on demand and cached by
modification time and length, so a file edited outside the app is read again on the next
search. There is no index and no file watcher. The `Mod+P` panel
(`GlobalSearchModal.svelte`) calls `global_search` and opens a hit with the location
helpers of `navigation/app-shell.ts`.

## Command surface

| Area         | Commands                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Hub          | `list_hub_skills`, `get_hub_skill`, `plan_import`, `import_skills`, `remove_hub_skill`, `sync_skill`, `diff_skill`, `check_source_updates`, `migrate_legacy`, `migration_status`                                                                                                                                                                                                                       |
| Install      | `install_skill`, `uninstall_skill`                                                                                                                                                                                                                                                                                                                                                                     |
| Scan         | `scan_folder`, `import_scanned`                                                                                                                                                                                                                                                                                                                                                                        |
| Instructions | `list_instructions`, `get_instruction`, `read_instruction`, `write_instruction`, `rename_instruction`, `import_instructions`, `create_instruction`, `detect_instruction_files`, `detect_instruction_github`, `install_instruction`, `uninstall_instruction`, `remove_instruction`, `sync_instruction`, `diff_instruction`, `list_instruction_files`, `read_instruction_file`, `write_instruction_file` |
| Projects     | `list_user_projects`, `add_user_project`, `update_user_project`, `remove_user_project`, `list_workspaces`, `add_workspace`, `remove_workspace`, `scan_workspace`, `register_projects`, `list_memory_files`                                                                                                                                                                                             |
| Search       | `global_search`                                                                                                                                                                                                                                                                                                                                                                                        |
| Favorites    | `list_favorites`, `set_favorite`                                                                                                                                                                                                                                                                                                                                                                       |
| Detection    | `detect_github_manual`, `detect_github_auto`, `detect_zip`, `detect_folder` (stage into a temp dir)                                                                                                                                                                                                                                                                                                    |
| Other        | marketplace, agent apps, projects, settings, library sync, file readers, translation                                                                                                                                                                                                                                                                                                                   |

Services live in the `youskill-core` crate (`src-tauri/crates/youskill-core`), which the
Tauri app and the `youskill` CLI (see [CLI.md](CLI.md)) both depend on; the app's
`src-tauri/src/commands/*` are thin adapters over it.
Services: `hub_service` (hub + lock records), `install_service` (targets), `drift_service`
(pure state computation), `diff_service` (file-level diff of the hub against a target or
source), `scan_service`, `migration_service`, `source_service` (network), `lock_service`
(atomic lock writes + process-wide operation mutex), `env.rs` (paths and agent/project
lists, injectable for tests).

## Migration from the previous layout

Before the hub, `~/.agents/skills` was the canonical directory and three lock files existed
(`~/.agents/.skill-lock.json`, `<config_dir>/youskill/native-skill-lock.json`,
`<project>/skills-lock.json`). On first list the app migrates lazily and idempotently:
hub copies are created, symlinks into the old canonical directory are relinked to the hub,
real copies become install records, and nothing legacy is deleted. The report is stored
under `migration` in the lock and shown once in the library.

A version 1 lock keys records by name and keeps the hub copy at `skills/<name>`.
`migration_service::migrate_skill_ids` runs before any listing, lookup, import or scan:
each such record gets a UUID, its directory moves to `skills/<id>/<name>`, its symlink
installs are relinked, and the lock is written, one record at a time. A run stopped after
the move finds `skills/<id>/<name>` without a record and keeps that id.

## Frontend

- `src/lib/api/hub.ts` mirrors the command surface (camelCase payloads).
- `src/lib/stores/hub.ts` holds the skill list (`hubSkillsById`; `skillLabel` adds the
  source to a name that more than one skill has) and agent apps. Selections, routes and
  dialogs carry skill ids. `stores/modals.ts` holds the
  import / install / force-confirm / diff modals and `performAction`, the helper that turns a
  blocked `ActionResult` into a confirmation.
- `src/lib/api/instructions.ts` and `stores/instructions.ts` do the same for the
  instruction library; the install, location, diff and remove dialogs take a `kind`
  (`skill` | `instruction`) and `confirmForce` is the shared force-confirmation helper.
- Anything that can be a symlink on disk carries its resolved target: `linkTarget` on an
  install view, a memory file and an agent instruction file, `link_target` (plus
  `link_broken`) on a skill directory entry. `SymlinkMarker.svelte` renders the marker and
  the hover that names the target; `list_skill_directory` reports links instead of
  following them blindly, so a broken one is listed and a loop cannot be walked twice.
  Memory and agent instruction files also carry `references`: the files a pure reference
  file imports (every non-empty line an `@path` import), shown by `ReferenceMarker.svelte`.
- Routes: `/` library (skill list plus skill detail), `/instructions` (instruction list plus
  detail with a content preview), `/projects` install scopes (scope list
  plus a scope detail with its agents and skills; `?scope=user` or `?project=<path>` selects
  one), `/market`, `/skills/hub/<name>` and `/skills/remote/<name>` file viewers,
  `/settings`, `/agent-apps`.
