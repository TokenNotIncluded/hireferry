# Data lifecycle and capacity

This inventory describes the implemented server and browser behavior. Defaults
come from [`src/config.rs`](../src/config.rs); deployment overrides belong in
`config.toml`. Public policy values in `[legal.values]` are commitments displayed
to users, **not** executable deletion schedules. In particular, financial,
security audit and conversation records are not discarded to make room.

## Background maintenance

[`src/maintenance.rs`](../src/maintenance.rs) starts with the `serve` command and
runs immediately, then every `maintenance.interval_minutes` (default 60; allowed
15–1440). It does not depend on a page visit or on feedback, CLI, HumanGrant or
webhook enablement. Expired staff invitations are also covered.

Each phase changes at most `maintenance.batch_size` rows per table (default 200;
allowed 1–1000). Onboarding and promotion expiry share one orders budget. Child
credentials and HumanGrant sessions are removed before their parents; a parent
with remaining children is left for another pass. Webhook events and endpoints
are removed only after their deliveries are gone. This prevents a bounded parent
deletion from cascading through an unbounded child history.

The worker acquires and releases the SQLite mutex separately for each phase and
does database work on the blocking executor. A full batch requests another pass
after one second; a drained pass waits for the configured interval. Failure in
one phase does not stop the other phases; errors retry after one minute. Logs
record the phase and changed-row count, without record contents. Cleanup uses
short statements rather than a transaction covering the whole database.

Housekeeping never runs `VACUUM`, removes backups, or scans arbitrary filesystem
directories. SQLite reuses freed pages; the database file may therefore remain
the same size after deletion. Any physical compaction needs an operator-controlled
backup and maintenance window.

## Disposable database state

| Data | Validity and default retention | Capacity / overflow behavior | Cleanup trigger and configuration |
| --- | --- | --- | --- |
| `feedback` | Open: 180 days from submission or actual reopening; resolved: 30 days from resolution | 10,000 records. New submissions fail explicitly with HTTP 503 at capacity; an unexpired report is never silently evicted | Central worker; `feedback.retention_days`, `resolved_retention_days`, `max_records` |
| `feedback_limits`, `feedback_dedup` | Counters retained for 24 hours; duplicate hashes for 600 seconds | Persistent limits: 5 submissions/account/hour; anonymous submissions share a service-wide 30/hour budget. Duplicate and rate checks survive restarts and feedback cleanup | Central worker; `feedback.max_per_account_per_hour`, `max_guest_per_hour`, `duplicate_window_seconds` |
| `sessions` | Ordinary browser sessions: 24 hours by default; bootstrap sessions are short-lived. Background cleanup retains expired rows for one extra day | Login throttles bound creation rate, not a global row count | Central worker; `session_hours` controls validity. Request-side opportunistic cleanup is also bounded |
| `auth_challenges` | Email: 600 seconds; Google/passkey challenges: 300 seconds; background expiry grace: one day | Login throttles and single-use challenge checks; no separate global row cap | Central worker plus bounded request-side expiry |
| `auth_limits` | One current window per key; rows older than 24 hours are disposable | Per-key counters replace old windows; keys age out independently of logins | Central worker and bounded opportunistic cleanup |
| `admin_bootstrap` | Single short-lived bootstrap credential | Singleton table | Central worker removes expired bootstrap state; consuming/replacing it is part of login |
| `cli_grants` | Authorization codes: 120 seconds; device grants: 10 minutes; background expiry grace: one day | 1,000 pending grants by default; explicit rejection when full. Authorization request and user-code attempt limits also apply | Central worker; `[cli_auth]` validity and creation limits; bounded request cleanup |
| `cli_authorizations`, `cli_tokens`, `cli_refresh_tokens` | Access tokens: 15 minutes; absolute refresh-family lifetime: 30 days; remove a family only 30 days after its absolute expiry | 10 active connections by default. Rotating tokens do not extend absolute family expiry | Central worker; `cli_auth.access_token_minutes`, `refresh_token_days`, `max_active_tokens`, `maintenance.expired_credentials_grace_days` |
| `api_tokens`, `admin_api_tokens` | Manual tokens: 30 days; remove after expiry plus the same 30-day grace | 10 active manual tokens per user / administrator. Expired tokens that still back a live refresh family remain until that family no longer needs them | Central worker; shared expired-credential grace |
| `hg_requests`, `hg_grants`, `hg_sessions` | Request lifetime: 600 seconds; maximum granted lifetime: 24 hours. Expired/revoked history remains 90 days | Default 12 requests/actor/hour and 600 requests/service/hour. History volume follows those limits and retention; no independent global row cap | Central worker; `[humangrant]` limits and `maintenance.human_grant_history_days` |
| `hg_nonces`, `hg_limits` | Nonces survive the signed replay window; request-rate history survives one hour | Actor/service rate limits bound requests. A still-live nonce cannot be reused; expired nonce replacement does not depend on sweeping the entire table | Central worker plus bounded request cleanup; rate queries exclude expired rows even if a backlog remains |
| `staff_invitations` | Invitation links: 7 days. Retain 30 days after acceptance, revocation or expiry | 100 pending invitations; 30 creations/hour; one invite per email/minute. Explicit rejection at the limit | Central worker; `maintenance.staff_invitation_history_days`; no page-triggered full-table deletion |
| `webhook_deliveries` | Delivered/cancelled: 30 days; failed or stale queued/processing: 90 days, measured from creation. A live processing lease is preserved | 1,000 pending/processing per endpoint; 5 attempts by default. Existing fanout records queue overflow in `dropped_events`; feedback never uses this loss policy | Central worker; `[webhooks]` retention and queue settings, including while delivery is disabled |
| `webhook_events` | Retained while any delivery references the event | Fanout uses endpoint queue limits. Orphan events are disposable | Central worker, bounded orphan deletion; event lookup index avoids repeated delivery scans |
| `webhook_endpoints`, `webhook_runtime` | Active endpoints remain; revoked endpoints may be removed after failed-history retention once no deliveries remain. Runtime row is a singleton | 5 active endpoints/user by default | Central worker; `webhooks.max_endpoints`, `failed_retention_days` |

Used CLI refresh-token hashes are security evidence: retaining them until the
whole family expires permits replay detection to revoke the family. Cleaning a
used hash merely because an access token has expired would weaken that protection.

The durable background queue is `webhook_deliveries`; there is no second task
queue or separate infinite retry archive. Attempts, transport timeout and response
body size are bounded (`max_attempts=5`, `request_timeout_secs=5`,
`max_response_bytes=16384`). Queue cancellation and exhausted-lease transitions
also operate in bounded batches. Claiming a queued item independently checks its
current endpoint/account/resource eligibility, so a large cancellation backlog
cannot cause an unauthorized remainder to be delivered.

## Records that must survive cleanup

| Data | Lifecycle and bounds | Retention / operator action |
| --- | --- | --- |
| `orders`, `payment_events`, `ledger`, `payment_refunds`, `refund_events` | Financial and payment-idempotency evidence. Pending orders are capped at 20/user by default; that is not a cap on historical payments | No automatic deletion or count-based eviction. Expired onboarding checkouts and promotional reservations change status in bounded background batches; checkout URLs are cleared, while gateway IDs and financial rows remain. A verified late onboarding payment can still settle exactly once |
| `promo_applications`, `promo_codes`, `promo_redemptions` | One pending application/user; code uses have explicit quotas; publication reservations expire after 24 hours | Application decisions and consumed/released redemption records remain. Expiry releases a reservation through the existing order-status trigger; it never replenishes consumed credit or deletes payment history |
| `audit`, `account_moderation` | Security and business audit; moderation state is one row/account | No automatic truncation. Feedback audit entries store metadata rather than duplicating the feedback text. Establish an archive/legal-hold policy before deleting security evidence |
| `users`, `jobs`, `matches`, `mandates`, `outreach` | Durable recruitment records. Matches are unique per seeker/job; mandates are one/user. Job deletion preserves associated financial records | No date-based eviction. Job deletion uses existing lifecycle rules, including clearing content on permanent deletion. Workbench pagination bounds response size, not stored history |
| `chat_conversations`, `chat_messages`, `chat_reads` | Business conversation history; message bodies are limited to 10,000 characters. Read state is one row per conversation/reader | No automatic message deletion. Recent-list defaults and page limits are query bounds, not storage caps; customer deletion/archival requires the applicable business and legal policy |
| `staff_members` | Staff membership history; at most 100 active members; one membership/account | Disabled memberships remain. Invitation cleanup never removes membership or its audit trail |
| `admin_passkeys`, `account_access_clock` | Operator-managed login keys; access clock is singleton state | Keys remain until explicitly revoked. No time-based deletion of valid credentials |
| Workbench FTS indexes and their shadow tables | Derived search state maintained by base-table triggers | They follow the retained base records; no independent history is appended |

These retained categories **do not currently have a global storage quota or an
automatic external archive**. Reporting them as fully bounded would be incorrect.
They require database-size/disk monitoring, tested backups and a reviewed archive
policy. A count cap that discarded payment idempotency keys, audit evidence or
messages would risk duplicate financial processing or destroy user records.
`legal.values.transaction_retention_years`, `support_retention_years` and
`access_log_days` alone do not implement such an archive.

## Browser memory, files and logs

Browser state is tab-local unless explicitly saved as a small preference. The
workbench keeps at most 50 recent query histories with 100 cursors each; older
history can restart from the first page. Mandate detail retains one record and
outreach retains the current bounded list. Job cache replacement keeps the
current list or detail. Sensitive caches are cleared when identity changes, and
late mandate responses cannot repopulate a cache after navigation or logout.
Chat drafts and uncertain sends are limited to 50 conversations each. At the
pending-send limit the UI rejects another conversation's send rather than
discarding an existing idempotency key; retrying an existing pending send remains
available. Current thread messages remain while that thread is open.

The server embeds static assets and does not maintain an upload spool or build
cache. Avatars are replaced in the user row; image uploads are limited to 2 MiB
and converted to a bounded thumbnail. The explicit `backup` CLI command writes
to the requested path; backups are not automatically deleted. Rust `target/`,
browser test reports and temporary development servers are workspace artifacts,
not application data. CI test/report artifacts expire after 14 days and CLI
build intermediates after 7 days in the existing workflows. Published releases
are deliberate durable artifacts.

Application tracing goes to stdout/stderr. The supplied systemd service uses
`PrivateTmp=true`; it does not create a separate application log file. Host
journald rotation and any reverse-proxy access-log retention are deployment
responsibilities; the supplied Caddyfile does not enable an access-log file.
Configure and verify host log byte/time limits independently of the public legal
template values. Housekeeping must not silently delete a deployment's logs.

For workspace cold storage, move only inactive historical builds, old reports or
recoverable caches into a project-specific folder. Verify the uploaded size and
cryptographic checksum before removing the local copy. Never remove uncommitted
source, the only recoverable copy or an in-use file. Google Drive authorization
does not imply that the running server has a Drive mount or a configured archive
job.

## Verification

[`tests/maintenance.rs`](../tests/maintenance.rs) exercises real SQLite foreign
keys, bounded passes, refresh replay history, HumanGrant state, staff invitation
retention, financial/audit/message preservation, late settlement after background
expiry, webhook leases and cancellation, error isolation, and the actual
background worker with all related features disabled. The worker test proves a
backlog larger than one batch drains without a request, even with its normal
interval set to one day.

[`tests/feedback.rs`](../tests/feedback.rs) covers feedback permissions, durable
limits/deduplication and capacity behavior. [`tests/service-ui.test.mjs`](../tests/service-ui.test.mjs)
checks cache eviction, identity/navigation changes, bounded cursor history and
preservation of pending-message idempotency. Run the repository CI commands for
the current revision; this document does not claim an unexecuted test passed.
