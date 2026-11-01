# Library sync: item scope and device files

Status: Accepted

## Context

Library sync backs up the library to one branch of a private GitHub repository through the GitHub REST API, without a local clone. Each item in `library.json` has `agents`, the user-level agents it is enabled for. This field merges per agent, so that agents that one computer does not have stay enabled. Shared content and the installs of each computer are in the same record. Each computer gets every item, and the user cannot keep an item on one computer only.

We compared three layouts:

- A. One branch for shared items (`main`) and one branch for each device.
- B. One branch, with the scope in the path (`shared/...` and `devices/<id>/...`).
- C. One branch, with the scope as a field of each item, and one file for each device.

Library sync does not use git merges. Each layout needs the per-item three-way comparison of the app. Thus branches give only separate refs. In A, a backup that changes a shared item must update two refs, and one update can fail after the other succeeded. Each device must also keep two bases. The main benefit of A is that each device writes only its own branch. B and C get the same result when a backup that is behind the head compares again against the new head (see below).

## Decision

- Use layout C. Keep one branch. Each item in `library.json` has a scope: `shared`, or device-only for one device id. A new item is shared.
- Scope is set per item: a skill, an instruction template, or a custom agent. It is not set per file.
- Install state belongs to a device. Each device has `devices/<device-id>.json` with its installs per agent. Shared item records do not have `agents`.
- A device gets the shared items and its own device-only items. It does not get the device-only items of other devices.
- A backup that finds the head moved since its base compares each item again against the new head. When no item changed on both sides, the backup commits on top of the new head without user action. Only an item that changed on both sides goes to the restore preview. A device file and the device-only items of a device have one writer, so they do not cause conflicts.
- Device-only is not private. Each person who can read the repository can read all items. Content that must stay private needs a different repository.
- [ADR 0003](0003-library-sync-id-on-first-merge.md) stays in effect. Ids are unique across all scopes.

### Edits and scope changes

- A local edit of a shared item changes the shared version, and each device gets it. To change an item on one device only, the user first copies it as a device-only item. The copy gets a new id.
- Each device can change a shared item to its own device-only item. On the other devices, the next restore shows the item as removed from the shared items, the same as a removal in the repository. An item unchanged there is removed by default. A device that keeps it gets a copy with a new id as its own device-only item.
- Only the owner device can change its device-only item to shared, or remove it. On the other devices, the next restore shows a new shared item as incoming.
- Other devices can read a device-only item and copy it as their own. The copy gets a new id.
- Custom agents have a scope per agent, the same as skills, and are shared by default. Restore compares them per agent, not as one `agents.json` file.

### Devices

- A device id is a random uuid that the device makes. The device name comes from the computer name and the user can change it. The name is in the device file. It is not an identity.
- On the first connect, the user starts a new device or takes over an existing device. A device that takes over gets the device id, the installs and the device-only items of that device.
- On the first connect, the user can copy the installs of another device. The default is the device with the latest backup. Only agents that exist on this computer are copied.
- To remove a device, the user selects for each of its device-only items: make it shared, give it to another device, or delete it. Then its device file is deleted. Git history keeps the old content.

### Migration and restore

- The new layout is `library.json` version 2. Releases up to 0.10.0 refuse a library with a version above 1, so they cannot restore from a migrated repository and do not write to it.
- The first device that syncs with the new layout migrates the repository after the user confirms. The confirmation tells the user that other devices must update the app before they can sync again. The migration is one commit: version 2, all items shared, no `agents` field, and the device file of this device.
- Each device makes its device file from its own local install records, not from the old `agents` values. The old values are a union over all devices.
- A device that was connected before the migration gets a new device id when it updates. It does not show the choice between a new device and take over. That choice is only on the first connect.
- A restore from an earlier commit changes the shared items and the items and device file of this device only. It does not change the device files or device-only items of other devices.
- The pending list stays. It comes from the device file of this device, after a restore, a take over, or a copy of the installs of another device.

### Delivery

- Fine-grained restore (issue #60) merges first. This layout uses its preview, checkpoint and undo.
- Step 1: layout version 2, device files, migration, the backup on a moved head, and the scope field. All items are shared, and the UI has no scope control.
- Step 2: scope changes on items, device management (name, take over, remove, copy installs), and copies of device-only items from other devices.

## Consequences

- A backup is one commit, and each device has one base.
- A scope change edits one field. The files of the item do not move, and its history continues.
- The per-agent merge of `agents` is removed. An install on one device does not enable the item on other devices.
- A backup writes the full tree, not a change on a base tree. Thus it must keep the device files and the device-only items of other devices from the head.
- When two computers use one device id, both write the same device file, and the rule that a device file has one writer stops being true. The user must take over only a device that is no longer in use.
- The code that selects the items of a device must be in one function with tests. An error in it can install a device-only item of another device.
- The scope is not visible in the paths of the repository. A person who reads the repository on GitHub must read `library.json`.
- The history of one device is not a separate branch. It is available from the commits that change its device file or its items.
- Existing repositories need a migration: `agents` moves into the file of the device that does the migration, and all existing items become shared.

## Reconsider when

- Users need content that only some devices can read. This needs more than one repository.
- Concurrent backups from many devices often fail to rebase on the head.
- Library sync starts to use a local clone and git merges.
