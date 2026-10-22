# WebMCP registration lifecycle — HF-QA-002

## Reproduction and cause

The unmodified SPA at `1345338c67f99237b3feff0f45ba66646bc3419b` was tested with synthetic administrator data on localhost in Chrome Headless Shell **151.0.7922.34**. Switching the homepage language, opening feedback, then switching language again produced **31 `Duplicate tool name` page errors**, with 31 tools still registered.

That browser exposes both `document.modelContext` and `navigator.modelContext`. Its `registerTool()` returns a Promise and its interface has no `unregisterTool()`. Registration is released using the `AbortSignal` supplied to `registerTool()`. The old synchronous `try/catch` swallowed the missing-method error, forgot the tool names, and did not handle rejected registration Promises. Chrome documents the current [registration and AbortSignal lifecycle](https://developer.chrome.com/docs/ai/webmcp/imperative-api).

## Ownership and identity

`static/webmcp-lifecycle.js` exports `createWebMCPOwner()`. Each instance owns only its own registrations and controllers:

```js
const owner = createWebMCPOwner();
await owner.sync({ scope: identityAndPermissionKey, tools, isCurrent });
await owner.clear();
```

`sync()` serializes registration/removal and handles asynchronous native errors. An unchanged identity and schema preserve registrations across language and route rendering. `clear()` immediately invalidates execution and aborts registrations, including registration that has not settled; its returned Promise waits for cleanup. Legacy implementations with `unregisterTool()` are also supported. Failed legacy cleanup remains tracked instead of losing ownership and causing repeated duplicate registrations.

`scope` and `isCurrent` describe identity/permission validity, not a render counter. Different modules use independent owners. A module that is usable only on a specific route includes route validity in its guard and clears its owner when leaving that route. Async catalog fetches must also check their request identity before calling `sync()`.

The SPA rechecks identity after session refresh and retains an epoch guard across explicit invalidation, even if the same identity key returns. Business callbacks check this guard before eligibility reads, after awaited reads, and before business requests. Server authorization remains authoritative. Manager-only sessions have no administrator browser-AI catalog; removing an explicit employee grant removes its tools. Fresh eligibility results remove or restore ordinary-user tools without giving an employer seeker-only tools.

## Verification

The regression uses the native browser API, not a replacement model-context registry. It observes registrations while forwarding them to Chrome. The same homepage reproduction now has **zero page errors**, with the expected 31 administrator tools.

- Eight language changes, open/closed feedback, and SPA routes preserve the catalog without re-registration.
- An actual native tool execution is revoked during an identity change; retained callbacks cannot issue business requests afterward.
- Employee removal, same-account seeker/employer changes, fresh eligibility changes, and logout produce the expected tool sets.
- A pending native registration is aborted before settlement. An independent owner remains registered when the workspace owner is cleared.
- Eight unit tests also cover asynchronous rejection, legacy async removal, failed cleanup, and context replacement. The existing 113 service-UI tests remain passing.

Run `node tests/webmcp-lifecycle.test.mjs` and the real-browser gate:

```sh
HFY_PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs \
HFY_BROWSER_EXECUTABLE=/path/to/chrome-headless-shell \
node tests/webmcp-lifecycle.browser.mjs
```

The browser gate enables `--enable-blink-features=WebMCP` and fails clearly if the native API is unavailable. It uses isolated localhost fixtures and does not access production sessions. The release CI browser suite includes this gate.
