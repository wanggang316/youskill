# Glossary

| Term | Meaning |
| --- | --- |
| Marketplace | The remote catalog used to discover and import skills. |
| Category | A server-managed tag. Its numeric tag id filters the skill list. |
| Hot repository | An enabled featured repository with type `hot`. It is curated, not a live popularity ranking. |
| Source | The repository key in `owner/repo` form used by the catalog filter. |
| Selection | The active tag, All skills, or one hot repository. Tag and repository filters are mutually exclusive. |
| Marketplace id | The server skill id used to open the exact remote entry. It is separate from the local hub id. |
| Browse state | Active tab, remembered selections, search, sort, loaded results, and scroll position for the current app session. |
| App update check | A check for a newer signed YouSkill release through the Tauri updater manifest. It is separate from a source update check. Avoid the bare word "update". |
| Source update check | A check of installed GitHub-sourced skills against their upstream repositories. It is separate from an app update check. |
| Device | One computer that is connected to the library sync repository. A device has a generated device id and a device file with its installs. |
| Item scope | Which devices get a library item: shared (every device) or device-only (one device). Device-only is not private: every reader of the repository can read it. |
| Shared item | A library item with the shared scope. Each device gets it. Its record has no install state. |
| Device-only item | A library item with the scope of one device. Only that device gets it. |
