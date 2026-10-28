# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking Changes

- The zip backup in Settings is replaced by GitHub backup. Existing `skills_backup_*.zip` files are left where they are.

### Added

- Back up the library to a private GitHub repository with the account selected in Settings > GitHub, and restore it on another computer. Synced: skills, instruction templates, the agents each one is enabled for at user level, and custom agents; install paths, projects, settings and keys stay on each computer. After a pull, Settings lists the installs this computer is missing and applies the selected ones. A backup or pull that would need merging changes from both sides is refused without changing anything.

### Changed

- Settings > Backup is now Backup & Sync.

### Fixed

### Removed

- The zip backup to a local folder.

## [0.9.15] - 2026-10-24

### Breaking Changes

### Added

- Instruction file groups in the Instructions list can be collapsed and expanded; the state is remembered ([HAN-261](https://linear.app/gumpw/issue/HAN-261)).
- Workspace groups in the "Install to" location picker can be collapsed and expanded; the state is remembered ([HAN-262](https://linear.app/gumpw/issue/HAN-262)).

### Changed

- Opening a skill in Market shows its detail in the content panel and keeps the category and repository list visible ([HAN-260](https://linear.app/gumpw/issue/HAN-260)).

### Fixed

- The update dialog hides release note sections that have no entries ([HAN-256](https://linear.app/gumpw/issue/HAN-256)).

### Removed

## [0.9.14] - 2026-10-22

### Breaking Changes

### Added

### Changed

### Fixed

- Recover empty skill names from existing hub directories so the library can display and read skills after an upgrade, without changing their ids or install history.

### Removed

## [0.9.13] - 2026-10-21

### Breaking Changes

- The library keys each skill by an id and stores it at `~/.youskill/skills/<id>/<name>` (lock file version 2). The first start moves the library and relinks symlink installs. An earlier version cannot read the new layout.
- The CLI `--json` output and the command line arguments use the new skill id: `list`, `show`, `import` and `update` report it, and every skill argument accepts an id, a source key or a unique name.

### Added

- Skills with the same name from different sources can be in the library together. Each one can be installed into a different place; an install into a directory that holds the other one asks before it replaces it.
- The import dialog shows library skills with the same name or source, and lets you replace one of them or add the import as a new skill. A zip import replaces the newest zip skill of the same name by default.
- `youskill import --replace <skill>` and `--add`; skill arguments accept source keys such as `owner/repo@main:tdd`.

### Changed

- Marketplace shows stars beside category result sources and in the hot repository header. Repository results omit the repeated source, and sorting uses an icon menu.

- Marketplace has category and hot repository navigation beside a scoped skill list. Search, sort, results, and scroll position are restored after viewing a skill.

- An import of a GitHub folder or local folder that is already in the library always updates that skill. The branch is part of a GitHub source, so a skill from another branch is a separate skill.
- The list column on the left of each page runs to the bottom edge of the window. Content panels keep their gap above the edge ([HAN-252](https://linear.app/gumpw/issue/HAN-252)).

### Fixed

- Marketplace tabs use a translucent sidebar track instead of an opaque panel background.

- Marketplace navigation uses the same hover and selection colors as the other sidebars and displays category and hot repository icons from the catalog.

- Category navigation works with older catalog servers that do not yet provide the tag summary endpoint.

- Marketplace details use the server skill id to distinguish entries with the same name. Pagination no longer stops one page early.

### Removed

## [0.9.12] - 2026-10-18

### Breaking Changes

### Added

- Homebrew install: `brew install --cask wanggang316/tap/youskill` for the app and `brew install wanggang316/tap/youskill` for the command line tool. The release workflow publishes both to `wanggang316/homebrew-tap` after each release.

### Changed

- On macOS, the top bar and list sidebars use the native translucent sidebar material, so the desktop shows through with a blur.

### Fixed

- Dragging the window by its top bar keeps the arrow pointer and no longer selects text.

### Removed

## [0.9.11] - 2026-10-16

### Added

- The SKILL.md preview shows every frontmatter field in a table above the body, in document order. Lists and tool patterns render as chips, nested maps such as `metadata` as key-value rows, and URLs as links.
- The Skill Library groups skills installed from GitHub by their source repository.

### Changed

- The app opens straight into a placeholder layout in your theme while the library loads, instead of a blank white window ([HAN-242](https://linear.app/gumpw/issue/HAN-242)).
- The update dialog has a shorter header.

## [0.9.10] - 2026-10-15

### Added

- Keyboard shortcuts for the top bar: `1` Skill Library, `2` Instructions, `3` Projects, `m` Marketplace, `n` Import, `⌘,` Settings. Hold `⌘` for half a second to show the keys in place of the icons, and hover a button to see its key next to its name. Change or reset the keys in Settings > Shortcuts ([HAN-239](https://linear.app/gumpw/issue/HAN-239)).

### Changed

- Rows in the Skill Library and Instructions lists no longer show a leading icon, and their text lines up with the group headers above them.

## [0.9.9] - 2026-10-14

### Fixed

- Markdown preview renders bold and italic text next to CJK punctuation, for example `**“禁止变通方案”**规则` in a translation ([HAN-235](https://linear.app/gumpw/issue/HAN-235)).
- A failed translation shows a notice above the file instead of failing silently. A result that is identical to the original text is treated as a failure, so a model that echoes the source no longer fills the translation cache with untranslated text ([HAN-236](https://linear.app/gumpw/issue/HAN-236)).
- The skill file viewer shows one "open in file manager" button instead of two.

## [0.9.8] - 2026-10-13

### Added

- An update dialog in the style of Sparkle. Check Update in Settings > About and the new Check for Updates… item below About YouSkill in the macOS app menu open it. It shows the app icon, the new version, the release notes, Remind Me Later and Install Update, or that you are up to date ([HAN-225](https://linear.app/gumpw/issue/HAN-225)).
- Remove for skills picked with Shift+click in the Skill library. The action bar shows Remove next to Install and Cancel. It opens one dialog for all picked skills, with the option to also delete their installed copies ([HAN-231](https://linear.app/gumpw/issue/HAN-231)).
- Retranslate in the translation view menu of a translated Markdown file, below a separator. It translates the file again without the cached translation, replaces the shown and cached translation, and keeps the selected view ([HAN-233](https://linear.app/gumpw/issue/HAN-233)).

### Changed

- Settings dropdowns are as wide as the selected option instead of one fixed width. All settings panes use the same content width. The OpenRouter API key field is below its label and uses the full card width, and the Chinese title of the translation pane is "翻译" ([HAN-226](https://linear.app/gumpw/issue/HAN-226)).
- The selected tab in segmented tabs (the skill and instruction detail tabs and the tabs in the install and import dialogs) slides to the new tab, the same as the top bar ([HAN-227](https://linear.app/gumpw/issue/HAN-227)).

### Fixed

- GitHub owner avatars in the Skill library no longer reload each time you open the library. The app keeps them in a disk cache for 7 days and in memory while it runs, and shows the GitHub icon instead of a gray circle while an avatar loads ([HAN-228](https://linear.app/gumpw/issue/HAN-228)).
- The confirm button of the remove and force dialogs is red again instead of the normal accent color.

## [0.9.7] - 2026-10-11

### Changed

- The sidebar is replaced by a top bar: the logo on the left, Skill library, Instructions and Projects as tabs in the center with Market and Import as icon buttons next to them, and Update and Settings on the right. The list pane next to the content panel can be resized by dragging its edge; double-click the edge to reset it.
- Settings use two panes: General, Agent Apps, Translation, GitHub, Backup, Command line tool and About. Agent apps, translation settings and the GitHub account are edited in place instead of in a separate page or dialog. Click the settings button again to go back to the previous page.
- A skill file that was translated before opens with its saved translation in the display mode set in translation settings. No new translation request is sent ([HAN-217](https://linear.app/gumpw/issue/HAN-217)).

### Fixed

- A GitHub skill no longer shows "update available" when nothing changed upstream. The marketplace index can lag behind GitHub; when its recorded version differs from the installed one, the app now confirms against GitHub before reporting an update, so a pull clears the notice instead of it coming back on the next check.

### Removed

- The separate `/agent-apps` and `/settings/github` pages. Their content is in the Agent Apps and GitHub panes of Settings.

## [0.9.6] - 2026-10-10

### Added

- The app checks GitHub-sourced skills for updates when it starts, when the library is opened from the sidebar and when the library is refreshed. All skills are checked in one batched call; automatic checks run at most every five minutes, the refresh button always checks.
- Skill file translation has a bilingual view: each original paragraph is followed by its translation. After translating, the file view switches between Bilingual, Translation and Original. Translation settings add "Display mode" (Bilingual by default, or Translation only) and "Translation style" (None by default, or Dashed underline) ([HAN-209](https://linear.app/gumpw/issue/HAN-209)).
- The skill library list is grouped: GitHub skills by repository owner (with the owner's avatar), local skills by the folder they were imported from, then skills without a source. Click a group header to collapse or expand it.
- Shift+click selects several skills in the library for a batch install. Skills that are already installed at the chosen location are reset there and installed again; the install dialog lists them.

## [0.9.5] - 2026-10-08

### Added

- `youskill` command line interface that manages the same hub as the app: `list`, `show`, `status`, `diff`, `import`, `install`, `uninstall`, `remove`, `update`, `sync`, `scan`, `agents`, `projects` and `instructions`, with `--json` output and exit codes for scripts. See `docs/CLI.md`.
- Shell completions for bash, zsh, fish, elvish and PowerShell through `youskill completions <shell>`.
- The app bundles the command line tool, and Settings offers to install it on the PATH. Releases also carry standalone `youskill` archives for macOS and Windows.

### Changed

- The app reloads its skill, instruction and project lists when its window regains focus, so changes made with the command line tool show up without a manual refresh.
- Hub operations are serialized across processes through `~/.youskill/.ops.lock`, so the app and the command line tool can run at the same time without interleaving changes.

## [0.9.4] - 2026-10-08

### Changed

- Markdown previews use a smaller, denser type scale: 14px body text, lighter headings, a quiet blockquote, and smaller code blocks and tables.
- The instruction editor fills the content area down to the bottom margin, instead of a fixed height.

### Fixed

- Markdown lists show their bullets and numbers again, nested lists use distinct markers, and task lists show checkboxes. Strikethrough, images, `kbd`, `mark` and `details` also get styles.
- Markdown tables keep the column alignment set in the source.
- Code blocks without a language are no longer colored by guessed syntax, and code block labels keep their capitalization.
- Action menus ("..."), right-click menus and agent hover cards stay inside the window: near the bottom of a list they open upward instead of going off screen. Menus also respond to the arrow keys and Enter.

## [0.9.3] - 2026-10-06

### Added

- Added a GitHub settings page that detects GitHub CLI (gh) accounts, lets you select the account to use, and uses it to download and update skills from private repositories.

### Fixed

- Projects marked "Not in project list" now have an actions menu. Before, they had no menu and could not be removed. The menu can add the project to the project list, or drop its install records so it is no longer listed. No file is touched.

## [0.9.2] - 2026-10-06

### Fixed

- Removing a workspace also removes its projects from the project list, and drops the install records of the ones whose folder is gone. Before, the projects stayed and moved to "Other", so the list looked unchanged after the removal. No file is touched. Projects left behind by a workspace removed in an earlier version are removed when the list loads.
- Projects in different folders can have the same name. Before, a project added later got a numbered name such as "translator 2"; these names are restored to the folder name. A project is now identified by its path.
- An error from a project list action now shows in the list when no location is selected.

## [0.9.1] - 2026-10-03

### Added

- Symlinks are marked wherever files and install targets are listed: the skill file catalog and viewer, install targets and the changes panel, the skills and memory files of an install location, and the instruction files list and detail. Hovering the marker shows the path the link points at, and a link whose target is gone is marked as broken.
- Instruction files that only import another file with Claude Code's `@` syntax (a `CLAUDE.md` holding `@AGENTS.md`) get a reference marker in the same places; hovering it shows the imported files, and the file detail lists them.
- Added registered scan paths: the library scan button now opens a panel listing the folders you scan for skills, with the last scan time and how many skills it found, plus the agent skills directories found on this machine as one-click suggestions. The panel also states the scan rules that were previously invisible: folders holding a `SKILL.md`, up to 6 levels deep, skipping `.git` and `node_modules`.
- Added a source filter to the Skill Library: all sources, local (imported from a folder or an archive), GitHub, or no source.

### Changed

- Scan results are now grouped by skill name and split into what the library does not have yet, what it already holds, and what needs no decision. Each folder carries one action instead of a checkbox plus a dropdown, and only one folder per name can go into the library, so several copies of the same skill can no longer overwrite each other in one import. The remaining copies can be recorded as install locations instead.
- Moved the scan and refresh buttons of the Skill Library into its header, next to the title.

### Fixed

- Listing a skill directory no longer fails when it holds a broken symlink, and a symlinked directory that points back at an ancestor is listed once instead of being walked forever.
- An instruction file that is a symlink (such as `CLAUDE.md -> AGENTS.md`) is listed as its own entry with the agents that read it, instead of being merged into the file it points at.
- Importing several folders holding the same skill name no longer breaks the result list with a duplicate-key error.

## [0.9.0] - 2026-09-10

### Breaking Changes

- Managed skills now live in a central hub at `~/.youskill/skills/<name>` tracked by a single `~/.youskill/.skill-lock.json`; `~/.agents/skills` is an ordinary install target (built-in "Agents (shared)" app). The previous lock files (`~/.agents/.skill-lock.json`, `native-skill-lock.json`, `<project>/skills-lock.json`) are read once for migration and no longer written.
- Installing is split into two steps: import (GitHub / archive / folder into the hub) and install (hub into user-level or project agent directories). The `list_skills`, `install_from_*`, `manage_skill_agent_apps`, `check_skill_version`, `check_skills_updates` and `delete_skill` commands were replaced by the hub commands.

### Added

- Added bundled LobeHub icons with initial fallbacks to the Agent Apps page.
- Added a two-column Skill Library: skill list with search and filters on the left, and a detail pane on the right showing source, version hash, an install matrix (user level plus one row per project with agent icons), a changes panel and the SKILL.md preview.
- Added drift detection: every install target is compared three-way against the hub (in sync / outdated / modified / conflict / missing / broken link), GitHub sources can be checked for updates, and folder sources are compared by content hash. Sync actions (update from source, push to targets, adopt a target, accept hub edits, push to source) refuse to discard local changes unless confirmed.
- Added folder scanning: scan a project, an agent's skills directory or any folder, see which skills are new, identical, or different from the hub, and import, adopt, overwrite or register them as install targets.
- Added automatic, idempotent migration of the previous layout on first launch, with a one-time report in the library.
- Added a Marketplace page; importing from it goes through the hub and then offers to install.
- Added Warp and Zed as built-in agent apps.
- Added workspaces: pick a folder, and YouSkill lists the projects inside it (a folder with an agent skills directory or an AGENTS.md / CLAUDE.md file) so they can be registered in one step; the project list groups projects by the workspace they came from and a workspace can be scanned again later.
- Added instruction-file paths to agent apps, at project level (`CLAUDE.md` for Claude Code, `AGENTS.md` for the rest) and at user level (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`); both are editable for custom agents, shown on the agent apps page next to the skills directories, and the project level is used to detect projects.
- Added a Memory section to each install location listing the instruction files that exist there and the agents that read them.
- Added grouping of agents that share a skills directory: the directory shows as a single agent (the shared directory as "Agents (shared)"), hovering lists the agents that read it, clicking it filters the skill list of the project to that directory, and installing or uninstalling covers the whole group.
- The skill detail lists where a skill is installed as projects: the user level is named after the local user and home folder, only projects that have the skill appear, and an add row opens a location list (user level and every project by workspace) where several places can be picked for one install, installed places are marked, and a right-click menu manages agents or uninstalls from that place.
- Added icons for Warp, Zed, VS Code, Augment, Command Code, Continue, Cortex Code, Crush, Droid, iFlow CLI, MCPJam, Neovate and Pochi, which used to show a letter.
- Workspaces in the project list can be collapsed by clicking their name; the choice is remembered, and a workspace opens again when one of its projects is selected from elsewhere.
- Added a Projects page for install scopes: the user scope and every project are listed on their own, and selecting one shows that location's status, the agents installed there (add an agent to install every skill of the location into it, remove one to uninstall them) and its skills, each with manage, diff, push, adopt and uninstall actions.
- Added a diff viewer for changed skills: compare the library copy file by file with an install target, with the source folder, or with the latest GitHub version.
- Added an Instructions page next to the Skill Library. It lists the agent instruction files (`AGENTS.md`, `CLAUDE.md`) found at user level and in every registered project, each viewable and editable in place, and the templates kept under `~/.youskill/instructions/<id>.md` (tracked by `.instruction-lock.json`, named freely, identified by a generated id). A template comes from local files or a folder, from a GitHub repository, folder or file URL, from an empty start, or from an agent file promoted to one; it can be renamed, edited (copies that were in sync follow the save), and installed to the file each chosen agent reads at user level or in a project (copy or symlink, agents that share a file are grouped); a file that already holds a different template is refused unless overwritten on purpose. Installed files get the same three-way drift as skills with push, adopt, accept and diff; the detail shows where a template came from, an agent file shows which template it belongs to, and the project detail's Memory section is now called Instructions and opens the file on that page.
- Added a resizable sidebar (drag its edge, double-click to reset; narrower than 140px it collapses to icons with small labels) and back / forward buttons next to the window controls, also on ⌘[ and ⌘].

### Changed

- Changed the default install mode to copy; symlink remains available per install.
- Changed the user-level skills directory of Codex, Cursor, GitHub Copilot, VS Code, Gemini CLI, OpenCode and Kimi Code CLI to the shared `~/.agents/skills` they read natively (Goose and Antigravity now use `.agents/skills` at project level); app presence is detected from the app's own config directory. Existing skills in the old app-specific directories are still migrated and scanned as targets of that app.
- Changed the skill detail pane into General and Skill files tabs: changes are listed above the install targets, each install location lists its agents as a compact icon row with target actions kept in the changes panel, and the file viewer with translation lives in the second tab.
- Changed backup to archive the whole hub (`~/.youskill`, skills and lock file).
- Changed GitHub imports to accept `/tree/<branch>/<path>` URLs and to record the branch that was downloaded.
- Changed the app to a wider two-column window with persistent navigation for adding skills, browsing the library, switching scopes, selecting projects, and viewing settings alongside the sidebar.
- Installing from a project page, from the user scope or from an install-target row no longer shows the scope switch: the skill goes where the action started, and the dialog title names that place. Only the library header, the market and the import flow still let you choose.
- The user scope is presented as the home directory, like a project: its path is shown, it can be opened in the file manager, and skill and memory paths are relative to it.
- Sidebar and header polish: the add action sits with the main navigation, the brand logo is gone, and single-line content headers are shorter.

### Fixed

- Fixed Agent Apps and Skill detail routes replacing the app shell instead of rendering in the right content pane.
- Fixed dragging the window by its sidebar or content header.
- Fixed archive extraction to reject entries that escape the destination directory.
- Fixed update checks for repositories whose default branch is not `main`.
- Fixed the agents of an install location being listed once per skill instead of once per skills directory.
- Fixed install targets naming only the agent that was picked at install time: a skills directory now lists every installed agent that reads it, so a skill in `~/.agents/skills` shows Codex, Cursor and the others that share it.
- Fixed install records keeping agents that no longer read their directory: listing now drops an agent id when the directory is neither that agent's skills directory nor one of its legacy ones.
- Fixed removing an agent tile on the projects page: it counted and targeted every directory that agent reads (a legacy Cursor folder offered to remove 18 skills). The tile now removes only its own directory, by path.
- Fixed projects whose folder was deleted or renamed lingering in the project list with nothing to do about it: the list and the project detail mark them, a menu on the row removes a missing project together with the install records that point into it (no file is touched), a registered project can be taken off the list, and scanning a workspace again lists its missing projects for removal alongside the new folders.

### Removed

- Removed the Global / project scope switcher; the library shows every hub skill, and install locations live on the Projects page.
- Removed the "take over unmanaged skill" permission setting and the version-pick dialog; scanning replaces both.
- Removed the project management dialog: projects come from workspaces now, and a workspace can be rescanned or removed from the project list.

## [0.8.5] - 2026-05-11

### Added

- Added a one-click bulk update action for all local skills with available updates.

## [0.8.4] - 2026-03-01

### Added

- Added one-click translation for Markdown files in Skill directories.

## [0.8.3] - 2026-02-28

### Added

- Added a skill detail catalog drawer with full skill file tree navigation and in-page file switching.
- Added skill detail file preview modes for code and images, with unsupported-format fallback actions.

### Fixed

- Fixed project-scoped local skill detail pages showing `Skill not found` by preserving scope and project path in detail navigation.
- Fixed scope resetting to `Global` after returning from skill detail by restoring scope/project state from return URL.

## [0.8.2] - 2026-02-27

### Added

- Added project skill management.

## [0.8.1] - 2026-02-24

### Added

- Changed the GitHub download method from git clone to direct download.
- Added 20 agent apps.
- Fixed compatibility issue when the local skills directory is absent.

## [0.8.0] - 2026-02-23

### Added

- Initial formal release of YouSkill
- Core desktop workflow for skill management
