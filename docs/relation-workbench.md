# Relation workbench: behavior and acceptance

The administrative People & roles page reads the relationship collection
independently of either directory's current page or scroll position. The
implementation is shared by the browser and the [six MCP tools](relation-workbench-mcp.md).
It preserves the existing candidate search, introduction, profile, job-management
and follow-up workflows.

## Views and interaction

| View | What is shown | How to narrow the scope |
| --- | --- | --- |
| Grouped overview | Exact user/job groups and bounded stage-colored bundles | Select a group, or choose different grouping fields |
| Focus | One fixed user or job and a page of its matching neighbors | Select a node on either side; stage and directory filters still apply |
| Matrix | Exact group-to-group relationship counts with stage composition | Select a nonempty cell to inspect that pair of groups |
| Directory | Independently paginated users and jobs, including unconnected records | Search/filter, then select an entity to focus |

Supported user groups are role, target industry and target roles; job groups
are status and employer. There are at most 20 groups on each side. Larger
catalogs retain 19 groups and an exact Other group, so aggregation does not
discard records. Changing a base search/filter clears obsolete group IDs.
User search accepts names, known email addresses, IDs and the stored intent
text. Compact responses do not return email addresses, resumes, job descriptions
or match notes.

Stable arrangement uses bounded group relationships to reduce crossings and
retains the previous order if the candidate arrangement is no improvement.
It does not reorder rows while scrolling. Name/degree ordering applies to the
complete group catalog or the **currently loaded node page**, as labeled in
the UI; it does not claim to globally sort all database records.

Introduced relations use blue solid lines, interviews purple dashes, hired
relations green solid lines and declined relations gray dots. Overview stage
bundles use slightly separate lanes so one stage cannot paint over all the
others. Labels and line patterns supplement color.

## Offscreen, hidden and unloaded are different

Node positions derive from stable row indices and scroll offsets, rather than
the presence of a DOM element. Only viewport rows and overscan are mounted.
When a relationship endpoint is above or below the list, its connection ends
at a counted boundary portal. Selecting that portal scrolls toward those
records. When both ends are offscreen, counts remain available without a
misleading line through the visible area.

Focus pages contain at most 100 neighbors. The page footer reports the full
matching total and records on subsequent pages; these are distinct from the
loaded page's offscreen relations. Portals do not imply that all 1,000 neighbors
were downloaded. The projection footer accounts for visible, offscreen,
hidden and unloaded endpoints.

Hiding lines or explicit entities changes presentation only. Graph counts and
business matches remain unchanged. Restore hidden reverses the exclusions;
larger exclusions belong in filters. Stage filters, independent directory
filters, group scopes and focus can be combined. Search/group changes explicitly
clear an incompatible focus rather than leaving a dangling selection.

Fullscreen uses a direct browser gesture, with an immersive layout fallback.
AI can request fullscreen but receives `requires_user_gesture` until a person
activates it. Narrow screens stack the directories and use accessible matrix
details; decorative connector lines are suppressed at those widths.

## Performance and consistency boundaries

- Canvas drawing is capped at 80 paths. The matrix and totals account for the
  entire selected collection even when fewer bundles are drawn.
- Directory/neighborhood requests are capped at 100 records. Scrolling within
  a loaded page does not query the database.
- Group ordering is bounded to the aggregate catalog. No full-graph layout,
  full-edge JSON transfer, force simulation or animation loop runs on every
  scroll event.
- Exact overview responses are cached per application/database, operator,
  normalized query and data revision: at most 32 entries and 4 MiB of serialized
  responses. Cache hits still check current authorization and revision.
- The graph HTTP handler and MCP/browser-tool invocations put blocking database
  work on the blocking executor. The application still has a shared SQLite
  connection mutex: a cold dense aggregation can delay other database users.
- Keyset cursors bind the operator, query and data revision. A data change
  returns `409 resync_required`; the client refreshes rather than combining
  pages from different versions. This is not historical MVCC.
- Browser acknowledgements are issued only after rendering the corresponding
  state. Delayed server replies cannot replace newer local interactions;
  presentation-only changes preserve scroll anchors.

### Explicit dense-case measurement

An isolated test inserted 1,000 seekers, 1,000 jobs and **1,000,000 actual match
rows**, with normal migrations, indexes and revision triggers. These are single
local measurements using a debug/unoptimized Rust build, not a production SLA.

| Operation | Measured duration | Serialized response |
| --- | ---: | ---: |
| Cold exact overview | 3,448 ms | 4,910 bytes |
| Same-query, same-revision cached overview | 638 microseconds | 4,910 bytes |
| 100-neighbor page | 151 ms | 56,772 bytes |
| 100-edge page, including the exact total | 2,169 ms | 29,347 bytes |

Cold overview was approximately 7,681 ms before this query optimization. The
remaining seconds-long cold aggregation is a real limit. Cache latency must not
be substituted for initial-query latency in product claims. MCP additionally
projects fields and enforces its response budget, so it need not expose the
browser's complete compact-node payload.

## Reproducible acceptance

```sh
cargo test --locked --workspace --no-fail-fast
node tests/people-jobs-ui.test.mjs
node tests/relation-graph-core.test.mjs
node tests/relation-webmcp.test.mjs
cargo build --locked
HFY_PLAYWRIGHT_MODULE=playwright node tests/admin-browser.mjs
HFY_PLAYWRIGHT_MODULE=playwright node tests/relations.browser.mjs
```

The real-server browser fixture creates an isolated temporary SQLite database
containing 1,000 users, 1,000 jobs and 10,900 synthetic matches, including one
1,000-neighbor focus and an unmatched user. It verifies bounded rows, offscreen
portals, zero graph requests on scroll, preserved scroll on line hiding, exact
totals, matrix drilldown, fullscreen, actual MCP-to-browser acknowledgements,
response byte budgets, and 390/320-pixel layouts. No production data or business
mutation is used. `HFY_BROWSER_SCREENSHOTS` optionally records labeled synthetic
screenshots; `HFY_BROWSER_EXECUTABLE` can select an installed Chromium.

The existing administrative browser gate also reproduces the synchronization
race deterministically: hold an old directory canonical response, select a new
focus, then release the old response. The newer focus must remain on screen.

The dense benchmark is deliberately opt-in:

```sh
cargo test --locked --test relations million_edge_benchmark -- --ignored --nocapture
```

The query and view suites separately check revocation, ownership, cache
isolation, group-count conservation, cursor tampering, byte-budget pagination,
atomic patches, retry IDs, focus membership, undo and activation receipts.
