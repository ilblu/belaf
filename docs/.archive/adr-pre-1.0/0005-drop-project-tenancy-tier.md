# ADR 0005 — drop the github-app `projects` tenancy tier

- **Status**: Accepted (belaf 3.0, github-app 3.0)
- **Date**: 2026-05-01

## Context

The github-app carried a multi-tenant hierarchy:

```
Workspace (billing, SSO, members)
  └── Project (name, slug, description, avatar, aggregated stats)
       └── Repository (per-repo settings, optional projectId FK)
            └── Release (the release event itself)
```

The dashboard rendered `/projects/$slug/{releases,packages,
automations,settings}` as the primary navigation surface. 17 routes,
6 components (project-avatar, -card, -menu, -stats, -tabs,
unassigned-repo-card), 10 + 3 API endpoints, and ~2200 LOC across
api + dashboard hung off the projects tier.

The user's audit found:

- Zero permission semantics on `projects`. Every auth check is
  `workspace.role === 'member'`. No `project_members` table,
  no project-scoped roles, nothing in Better-Auth.
- Zero billing semantics. Plan + limits are workspace-scoped.
  Projects have no tiers.
- Zero settings table. `repositories.settings` JSONB exists per-repo;
  project-level settings are empty.
- The CLI never knows about projects. No `projectId` in any
  `/api/cli/*` endpoint. CLI maps repo-by-name; projects are pure
  dashboard concept.
- Webhook dispatch is repo-scoped already. Project was just a lookup
  intermediary: `repo → project → automations`.

What `projects` actually did: UI grouping in the dashboard, container
for `post_release_actions`, denormalised stats cache (`totalReleases`,
`totalPackages`, `lastReleaseAt`).

## Decision

Drop the `projects` tier completely. Two-tier tenancy:
**Workspace → Repo → ReleaseUnit → Release.**

Concrete changes:

- DROP TABLE `projects` (CASCADE).
- DROP `repositories.project_id` column + index.
- ADD `repositories.tags text[] NOT NULL DEFAULT '{}'` (Wave 4
  step 8) as the new UI grouping mechanism.
- `post_release_actions.project_id` → `repository_id` (FK to
  `repositories`, ON DELETE CASCADE). Webhook dispatch simplifies
  from `repo → project → automations` to `repo → automations`.
- Delete `/api/projects/*` (10 primary + 3 nested action endpoints).
- Delete `/projects/$slug/*` (17 dashboard routes).
- Delete 6 project-aware components.
- Delete `useProjects` / project mutations / `Project*` API types
  on the dashboard.

Repo-tags (`repositories.tags: string[]`) replace the projects-tier
UI grouping role. Wave 4 step 8 ships:

- Free-form per-repo tag list (no schema for the tag value).
- Tag-filter chip strip in workspace overview.
- `<RepoTagBadge>` component — chip with active/inactive states.

Workspace-level "automation templates per tag" (apply automation to
every repo with tag X) is a 3.1-deferred follow-up; ship-1 covers
passive tagging + filter only.

## Consequences

- Dashboard's primary nav is the workspace overview. No
  `/projects/foo/*` URLs anywhere.
- ~1700 LOC removed (close to the ~2200 LOC audit estimate; some
  ancillary code stayed because it wasn't actually project-aware).
- Permission model unchanged: workspace.role is still the only check.
- If multi-team permissions ever become a need: add a `Team` table
  as a permission boundary inside Workspace. Non-breaking, easier
  than re-adding a tier.

## Migration

Acceptable as a destructive drop+recreate per the user's explicit
approval — no production data. Drizzle migration `0003_drop_projects_
tier_v3.sql` runs as a single atomic step.

## Alternatives considered

- **Rename projects → workspaces** (folding both tiers). Rejected:
  the workspace concept stays separate and meaningful (top-level
  tenant, billing, SSO).
- **Keep projects, just rename CLI's `Project` primitive**. Rejected:
  the audit showed projects is vestigial, and the shared name was
  permanent cognitive tax.
- **Drop projects, no replacement grouping**. Rejected: a workspace
  with 28 repos (clikd-shape) and no grouping option is a UX
  regression vs. 2.x. Repo-tags fill the gap.

## Why now (3.0) and not later

The 3.0 plan already touched the manifest wire format, the dashboard
schema parser, and the project/release-unit primitive split on the
CLI side. Doing the projects-tier drop in the same window means one
coordinated DB migration and one coordinated dashboard rewrite.
Splitting it across releases would mean shipping 3.0 with vestigial
projects-tier code still present and revisiting the entire stack in
3.1.
