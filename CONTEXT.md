# YouSkill

YouSkill is a desktop manager for agent skills and agent instructions. It keeps one
canonical copy of each item and installs it into the agent apps that read it.

## Language

### Library

**Hub skill**:
A managed skill whose canonical copy lives in the hub. The Library lists hub skills.
More than one hub skill can have the same name; the skill id tells them apart.
_Avoid_: Local skill, managed copy

**Skill id**:
The permanent identity of a hub skill. It never changes, also when the skill is renamed.
The one exception is the first library sync merge, where a local skill takes the id of
the same skill in the backup repository (see `docs/decisions/0003-library-sync-id-on-first-merge.md`).
_Avoid_: Skill key

**Skill name**:
The name that agents use for a skill and the name of its install directory. It is not unique.
_Avoid_: Skill id

**Source key**:
The identity of where a hub skill came from, such as one GitHub repository folder. One
source key gives at most one hub skill.
_Avoid_: Source id

**Instruction template**:
A managed Markdown file in the instruction library that can be installed as an agent file.
_Avoid_: Prompt, memory

**Agent file**:
An instruction file that an agent app reads in place, such as `CLAUDE.md` or `AGENTS.md`.
_Avoid_: Profile, memory file

### Scopes

**Project**:
A registered folder that agent apps work in. It is one install scope.
_Avoid_: Repo, workspace

**Workspace**:
A folder that holds projects and is scanned to discover them.
_Avoid_: Project root

### Navigation

**Global Search**:
One app-wide search, opened with a shortcut (`Mod+P` by default), that finds hub skills,
instruction templates and projects (with the User scope) by name or content and opens the
selected one on its own page. The UI calls it "Search" (搜索).
_Avoid_: Quick Open, command palette

**Filter**:
A narrowing of the list on the current page only, such as the Library source filter.
_Avoid_: Search

### Library sync

**Restore plan**:
The item-by-item comparison of the base (last synced commit), this computer and a target
commit, computed before a restore. A restore applies one plan with the user's decisions.
_Avoid_: Pull preview, diff

**Conflict**:
An item that changed on this computer and in the repository to different results. The user
keeps the local version, takes the repository version, or keeps both.
_Avoid_: Divergence

**Removal candidate**:
An item that the repository removed and this computer did not change. It is removed here
only after confirmation.
_Avoid_: Remote deletion

**Deferred item**:
A repository version the user left out of a restore. Backups carry it forward unchanged
until the user takes it or the repository changes it again.
_Avoid_: Skipped item

**Checkpoint**:
The local copy of the lock files, sync state and changed hub files taken before a restore.
Undo puts it back; the repository is not changed.
_Avoid_: Snapshot, backup

**Device**:
One computer connected to the library sync repository. It has a random device id and a
device file with its installs. The device name is for display only (see
`docs/decisions/0004-library-sync-item-scope.md`).
_Avoid_: Machine, host

**Item scope**:
Which devices get a library item: shared (every device) or device-only (one device).
Device-only is not private: every reader of the repository can read it.
_Avoid_: Visibility, privacy

**Shared item**:
A library item with the shared scope. Each device gets it. Its record has no installs.
_Avoid_: Common item, main item

**Device-only item**:
A library item with the scope of one device. Only that device gets it and only that device
can change its scope or remove it.
_Avoid_: Local item, private item

### Favorites

**Favorite**:
A marketplace skill that a person marks to revisit, identified by its marketplace id. A hub
skill is not a favorite. Favorites sync across devices through a server account; see
`youskill-web/docs/decisions/0001-user-accounts-for-favorite-sync.md`.
_Avoid_: Bookmark, collection, star

**Local favorite**:
A favorite stored only on this device. It needs no account and merges into the account at
sign-in.
_Avoid_: Guest favorite

**Account**:
A youskill server account that a person signs in to with GitHub or Apple. A GitHub and an
Apple sign-in are two accounts. Sign-out removes the favorites of this device; a session the
server revoked keeps them. See `youskill-web/docs/decisions/0003-client-sign-in-sessions.md`.
_Avoid_: Profile, user
