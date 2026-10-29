# Library sync: item ids on first merge

Status: Accepted

## Context

A skill id is the permanent identity of a hub skill, and an instruction template id is the permanent identity of a template. Library sync keys every item in the backup repository by this id. Two computers that import the same skill before they connect to the same repository give it two different ids. A first merge must match these items, or the library gets duplicates. After the match, one of the two ids must go.

The repository is the shared library. Other computers that already synced use its ids in their lock files, install records, and `.sync-state.json`. A change of a repository id shows on each of them as one removal and one addition. A change of a local id stays on one computer.

## Decision

- The id that is already in the backup repository wins. The local item takes it.
- This is the only case in which an id changes. It happens once per item, on the first merge of a computer with a repository that has a library (no last synced commit).
- A first merge pairs a local item and a remote item of the same kind when their content hash is equal, or when their source key is equal. An equal hash makes them the same item. An equal source key with a different hash makes a conflict.
- A rekey moves the local lock record and the hub directory or template file to the remote id. It keeps the name, install records, and install paths.
- Replace remote is an explicit choice of the local library. In this mode local ids stay, and the repository items that the local library replaces are removed from the repository.
- When the same id changed on both sides and the user keeps both, the remote version keeps the id. The local version gets a new id and a suffixed name. If the two versions have the same source key, the local version loses its source, because one source key gives at most one hub skill.
- The install records stay with the id, which gets the remote version. The local copy starts without installs. When a source-key pair is kept both, the two items already have different ids: no rekey, and the local item loses its source.

## Consequences

The glossary exception for ids is limited to the first merge. Code outside library sync can continue to treat ids as permanent. A rekey must update every record that refers to the old id in one operation, under the operation lock, with a checkpoint from which the user can undo it. A detached copy from "keep both" no longer gets source updates.

## Verification

Test a first merge with equal hashes (rekey, installs kept), equal source keys with different hashes (conflict), unrelated items (both kept with their own ids), replace remote (local ids kept), and keep both on one id (remote id kept, local copy rekeyed and detached when it shares the source key).
