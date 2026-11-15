# Project in-place detail

Status: Accepted

## Context

The project detail lists the instruction files and the hub skills of one project. A click on an item calls `goto` and opens the Library or the Instructions page. The user loses the project context and must navigate back by hand. The marketplace already shows a skill detail in place, in the content panel, with a Back button (see `0001-marketplace-navigation.md`).

## Decision

- A click on a hub skill or an instruction file in the project detail opens its detail in the content panel of the Projects page. The project list stays visible. The app does not go to another page.
- The in-place detail has the same functions as the detail on the Library or Instructions page: install, uninstall, sync, remove, and file edit.
- Do not make a second implementation. Move the detail actions out of the Library and Instructions routes into shared detail containers. All three pages use them.
- The URL holds the state: `/projects?project=<path>&skill=<id>`, `&file=<path>`, or `&instruction=<id>`. The User scope uses `?scope=user` in place of `?project=`.
- There is one level only. A link in the detail, such as "open template" in an instruction file detail, replaces the detail in place. Back always goes to the project detail.
- Back restores the scroll position of the project detail. The detail closes when the user selects another project, when the skill is removed, or when its install in this project is removed.

## Consequences

The Library and Instructions routes become thin hosts of the shared containers. `app-shell.ts` parses the new query parameters on the Projects page. A refactor that changes the project detail back to `goto` breaks this decision.

## Reconsider when

The detail needs a navigation stack deeper than one level, or the Projects page gets too narrow to show a full detail.
