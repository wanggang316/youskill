# Development Rules

## First Message

If the user did not give you a concrete task in their first message,
read `README.md`, then ask which area to work on. Based on the answer, read
the relevant docs in parallel.

- README.md
- docs/ARCHITECTURE.md (storage layout, lock file, drift model, command surface)
- docs/CLI.md (command line interface, crate layout, cross-process lock)

## Code Quality

### Requirements

- All code must compile without errors (`npm run build`)
- No linter warnings for modified code
- Add tests for new functionality when possible
- UI must use theme tokens from `src/app.css` (no hardcoded colors)
- Remove clearly unused code or assets

### Check Methods

- Frontend: `npm run build`
- Tauri: `npm run tauri:build`

## Commands

- `npm run format`: Apply frontend and backend formatting.
- `npm run check`: Run frontend checks and backend strict checks (`cargo fmt --check` + `clippy -D warnings`).
- `npm run test`: Run backend unit tests (`cargo test --manifest-path src-tauri/Cargo.toml --workspace`).
- Preferred local workflow after code changes: `npm run format` -> `npm run check` -> `npm run test`.
- Before commit/PR, ensure `npm run check` and `npm run test` both pass.

## Style

- Keep answers short and concise
- No emojis in commits, issues, PR comments, or code
- No fluff or cheerful filler text
- Technical prose only, be kind but direct

## Changelog

Location: `CHANGELOG.md` (if present)

### Format

Use these sections under `## [Unreleased]`:

- `### Breaking Changes` - API changes requiring migration
- `### Added` - New features
- `### Changed` - Changes to existing functionality
- `### Fixed` - Bug fixes
- `### Removed` - Removed features

### Rules

- Before adding entries, read the full `[Unreleased]` section to see which subsections already exist
- New entries ALWAYS go under `## [Unreleased]` section
- Append to existing subsections (e.g., `### Fixed`), do not create duplicates
- NEVER modify already-released version sections (e.g., `## [0.12.2]`)
- Each version section is immutable once released

### Attribution

- **Internal changes (from issues)**: `Fixed foo bar ([#123](https://github.com/org/repo/issues/123))`
- **External contributions**: `Added feature X ([#456](https://github.com/org/repo/pull/456) by [@username](https://github.com/username))`

## Adding a New LLM Provider (packages/ai)

## Releasing

## **CRITICAL** Tool Usage Rules **CRITICAL**

- NEVER use sed/cat to read a file or a range of a file. Always use the read tool (use offset + limit for ranged reads).
- You MUST read every file you modify in full before editing.

## **CRITICAL** Git Rules for Parallel Agents **CRITICAL**

Multiple agents may work on different files in the same worktree simultaneously. You MUST follow these rules:

### Committing

- **ONLY commit files YOU changed in THIS session**
- NEVER use `git add -A` or `git add .` - these sweep up changes from other agents
- ALWAYS use `git add <specific-file-paths>` listing only files you modified
- Before committing, run `git status` and verify you are only staging YOUR files
- Track which files you created/modified/deleted during the session

### Forbidden Git Operations

These commands can destroy other agents' work:

- `git reset --hard` - destroys uncommitted changes
- `git checkout .` - destroys uncommitted changes
- `git clean -fd` - deletes untracked files
- `git stash` - stashes ALL changes including other agents' work
- `git add -A` / `git add .` - stages other agents' uncommitted work

### Safe Workflow

```bash
# 1. Check status first
git status

# 2. Add ONLY your specific files
git add path/to/file

# 3. Commit
git commit -m "type(scope): description"

# 4. Push (pull --rebase if needed, but NEVER reset/checkout)
git pull --rebase && git push
```

### If Rebase Conflicts Occur

- Resolve conflicts in YOUR files only
- If conflict is in a file you didn't modify, abort and ask the user
- NEVER force push

## Agent skills

### Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
