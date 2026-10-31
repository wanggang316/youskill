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
