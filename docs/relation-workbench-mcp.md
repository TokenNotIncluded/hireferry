# Relation workbench tools and shared state

The relation workbench exposes six compact tools through the existing MCP
`2025-11-25` server. The authenticated browser uses the same descriptors and
implementations. These tools read relationship data and change presentation
state. Existing administrative match tools remain responsible for business
changes.

## Tool surface

| Tool | Purpose | Default response |
| --- | --- | --- |
| `hf_view_open` | Get the active view, read an existing view, create a view, or activate an existing view | Summary and versions |
| `hf_view_patch` | Apply an atomic batch of presentation commands with compare-and-swap and retry protection | Operation receipt |
| `hf_graph_query` | Run a bounded graph query and retain its query/revision as an owned result handle | Summary, no records |
| `hf_graph_read` | Read a projected page from a result handle | At most 20 rows and 8 KiB |
| `hf_view_save` | Save an independent named copy of a view | Saved view ID |
| `hf_view_list` | Find this operator's views and active view ID | At most 20 entries |

`GET /api/admin/relations/tools` returns the authoritative six descriptors as
`{"tools":[...]}`. Input objects reject unknown properties. The schema is kept
small and shared by MCP and WebMCP rather than maintained in two catalogs.

### Open, resume, create, and activate

`hf_view_open {}` gets or creates the logical operator's active view. It does
not replace an existing active configuration. `response:"state"` includes the
configuration when it is needed.

`mode:"resume", view_id` only reads a view. It never changes the active view or
increments `view_rev`; use it after a compare-and-swap conflict.

`mode:"create", operation_id` creates and activates a new view. Optional
`preset_id` clones a saved view. `mode:"activate", view_id, operation_id` instead
activates that existing view without copying it or changing its revision. An
activated saved view can be edited directly. Switching active views clears the
target's old render acknowledgement. This prevents a previous activation from
being reported as already rendered and allows repeated preset use without
consuming additional view slots.

The per-operator cap is 64 stored views. The initial API has no deletion or
renaming tool; activate and edit an existing view when the cap is reached.

## Presentation state and commands

The state contains `mode`, `filters`, `grouping`, `ordering`, `edge_display`,
`focus`, `hidden`, `presentation`, and `fullscreen_requested`.

| Command | Effect |
| --- | --- |
| `set_filters` | Replace the filter object |
| `set_grouping` | Set left and right grouping |
| `set_mode` | Select `overview`, `focus`, `matrix`, or `list` |
| `set_ordering` | Select `stable`, `name`, or `degree` |
| `set_edge_display` | Set `auto` for the selected view mode, or `hidden` to suppress all lines |
| `set_focus` | Focus a typed left/right entity, or clear with `null` |
| `set_visibility` | Hide or show 1–100 explicit IDs on `left`, `right`, or `edges` |
| `set_presentation` | Select `standard` or `immersive` application layout |
| `request_fullscreen` | Ask the browser to expose its fullscreen action |
| `undo` | Restore the previous presentation state once |

`stable` preserves ordering during scrolling and may improve crossings when a
scope is loaded. Ordering operates on groups in overview/matrix and on the
loaded page in focus/list; it is not a global alphabetical sort of the entire
database. Use `set_mode` to choose focused relationships or a grouped overview;
line visibility is a separate `auto`/`hidden` control. Early checkpoint values
`focused` and `bundled` are accepted as aliases of `auto` so saved views remain
readable; they are not advertised as additional rendering features.

Left grouping supports `role`, `industry`, and `roles`; right grouping supports
`status` and `owner`. Supported filters are the graph endpoint's typed user/job
searches, roles, account/job/match status, owner, explicit entity IDs, and group
IDs. A filter command replaces the previous filter object rather than merging
hidden client state.

Hiding a node or edge only changes this view. It does not delete a match, change
business status, or change graph totals. Querying the view reuses its business
filters and grouping; the `hidden` lists remain presentation-only exclusions.
Each hidden bucket holds at most 200 IDs, and serialized state is bounded to
32 KiB. Use a group or entity filter for a larger exclusion.

### Atomic changes, revisions, and retries

A patch requires the current `if_view_rev`, a unique `operation_id`, and 1–20
commands. Commands are evaluated together in a SQLite transaction. The final
focus must exist and belong to the final filter scope; otherwise the whole
batch fails with 404 or `409 excluded_by_filter` and the revision is unchanged.
For example, change filters and clear an excluded focus in the same batch.

A changed batch increments `view_rev` once. A no-op does not increment it. Undo
must be the only command and consumes the one stored previous state. Graph
`data_rev` is independent: presentation changes do not change it.

The server retains per-operator operation receipts for 24 hours. Retry the same
operation ID with the same body to receive the original result. Reusing that
ID with a different body returns `409 idempotency_conflict`. A cached receipt
describes the original operation; resume the view for its current UI status.
Authorization is checked again before returning cached results.

## Queries, handles, and token budgets

`hf_graph_query` accepts an explicit `query` or a `view_id`. An explicit query
replaces the view-derived query. The supported modes are `overview`, `nodes`,
`neighbors`, and `edges`; it does not expose an unbounded whole-graph dump.

The default response returns only a summary, a `result_id`, `data_rev`, and
expiry. `response:"rows"` returns the first projected page immediately.
`hf_graph_read` is already an explicit request for records, so it returns rows
without another summary-only round trip.

Handles expire after 15 minutes and are bound to the logical operator. They
retain the query and revision, **not a historical database snapshot**. Each
read rechecks current authorization and data revision. If underlying users,
jobs, mandates, or matches change, an old handle fails with `409
resync_required`; issue a new query. Expired or inaccessible handles return
404. At most 128 unexpired handles per operator are retained.

Read pages have `total`, `returned`, `remaining`, and `next_cursor`, including
when the response is shortened to fit a byte budget. Default limits are 20
rows and 8 KiB, with maxima of 100 rows and 32 KiB. `max_bytes` bounds the
serialized structured response body, not the surrounding MCP JSON-RPC/text
envelope. Select at most 16 fields from the advertised allowlist; `id` and
`kind` are always included. Sensitive long-form fields such as resumes, notes,
job descriptions, and email addresses are not in this allowlist.

A cursor is bound to its result and field projection; the graph cursor also
binds the authorized query and revision. Keep fields unchanged while paging.
The server reduces page length and reruns the same starting cursor to satisfy
the budget, so omitted records are not skipped. If even one projected record
and the cursor envelope cannot fit, the request fails explicitly with
`response_budget_exceeded`.

The first implementation deliberately provides no historical delta protocol,
set-algebra query, or fabricated graph checkpoint. Those features require a
separate database and authorization design before they can be promised.

## Browser synchronization and fullscreen

| HTTP endpoint | Use |
| --- | --- |
| `POST /api/admin/relations/views` | Open, create, or activate through `OpenInput` |
| `GET /api/admin/relations/views/active` | Poll the active view |
| `GET /api/admin/relations/views/{id}` | Read or poll an owned view |
| `PATCH /api/admin/relations/views/{id}` | Atomic presentation patch |
| `POST /api/admin/relations/views/{id}/ack` | Human browser render acknowledgement |
| `GET /api/admin/relations/tools` | Shared tool descriptors |
| `POST /api/admin/relations/tools/{name}` | Invoke one of the six shared tools |

A poll may send `view_id` and `after_rev`. If both still match, the server
returns `unchanged:true` without resending state; it still reports `data_rev`.
The browser polls while mounted, applies new state, loads its graph data, and
acknowledges the rendered `view_rev` with its `client_id`. Polling updates
liveness but is never an acknowledgement by itself. AI callers, including
WebMCP requests, cannot call the acknowledgement route successfully.

UI status is `stored` for inactive views or browsers absent for over 45 seconds,
`queued` for an active live browser that has not acknowledged the current
revision, and `applied` after its current-revision acknowledgement. These are
presentation receipts, not guarantees that a human has inspected the screen.

Immersive presentation is ordinary application layout. Native browser
fullscreen requires user activation. An AI request can set
`fullscreen_requested`; it reports `requires_user_gesture` until a person uses
the browser's fullscreen action and the browser acknowledges the resulting
state. No tool bypasses that browser restriction.

All routes use the platform's existing operator authorization and write-side
CSRF checks. Root-admin browser sessions and root-admin AI credentials share
the platform root's views. Each team member's browser/session and assigned AI
token map to that member's own views. Different team members cannot use another
member's view or result ID, and token revocation takes effect on the next
request. A handle is an identifier, never an authorization capability.

## Short interaction examples

Get the active view, then atomically select a matrix and immersive layout:

```json
{"name":"hf_view_open","arguments":{}}
```

```json
{
  "name":"hf_view_patch",
  "arguments":{
    "view_id":"<returned view_id>",
    "if_view_rev":1,
    "operation_id":"matrix-1",
    "commands":[
      {"op":"set_mode","mode":"matrix"},
      {"op":"set_presentation","presentation":"immersive"}
    ]
  }
}
```

Inspect a small page of candidate neighbors without serializing the view:

```json
{
  "name":"hf_graph_query",
  "arguments":{
    "query":{"mode":"neighbors","focus":{"side":"left","id":"<candidate_id>"}},
    "response":"rows",
    "fields":["id","label","edge_id","match_status"],
    "max_bytes":4096
  }
}
```

Save a named configuration and later activate it without creating more copies:

```json
{
  "name":"hf_view_save",
  "arguments":{
    "view_id":"<view_id>","if_view_rev":2,
    "name":"Review hiring matches","operation_id":"save-review-1"
  }
}
```

```json
{
  "name":"hf_view_open",
  "arguments":{
    "mode":"activate","view_id":"<returned preset_id>",
    "operation_id":"activate-review-1"
  }
}
```
