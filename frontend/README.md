# HireFerry message renderer

The Rust server embeds the checked-in `static/chat-renderer*` assets. Node is only
needed to rebuild or test them, not to serve messages. All runtime code and KaTeX
fonts are hosted by HireFerry. The site CSP does not need external script/font
origins, `unsafe-eval`, or frame permissions.

```sh
npm ci --ignore-scripts
npm run build:chat
npm run check:chat
npm run test:chat
```

`check:chat` reproducibly rebuilds in memory and compares all generated assets.
Run it before compiling Rust (which embeds the assets through `include_dir`).
Lifecycle scripts are unnecessary and intentionally disabled.

## Integration API

Load `/static/chat-renderer.css`, then lazily import the JavaScript once:

```js
const { mount } = await import('/static/chat-renderer.js');
const view = mount(messageBodyElement, {
  body: message.body,
  format: message.format, // plain, markdown, openui
  status: message.status, // streaming, complete, cancelled
});
view.update({ body: accumulatedBody, status: 'streaming' });
view.update({ status: 'complete' });
// Before removing the message element or navigating away:
view.unmount();
```

Pass the full accumulated body, not the latest delta. Updates batch into a frame
and keep the same React root. `mount` on an already mounted element reuses its
handle. The module also exposes `window.HireFerryChatRenderer`.

OpenUI controls emit a bubbling `hireferry:chat-action` DOM event:

```js
messageBodyElement.addEventListener('hireferry:chat-action', ({ detail }) => {
  // detail = { type: 'draft', text, formState }
  // Put text and readable scalar form fields into the composer for review.
  // Do not send automatically.
});
```

Alternatively pass `onAction(detail)` to `mount`. Use one integration mechanism
to avoid handling the same action twice. Form state is bounded to 32 scalar fields;
hidden action context is discarded. State lives in the current message island;
it is not persisted across a full page reload by this module.

## Supported content

- **plain**: escaped text and preserved line breaks.
- **markdown**: CommonMark, GFM tables/task lists/strikethrough, links, same-origin
  or raster data images, code blocks, and inline/display KaTeX formulas. Code
  highlighting supports JavaScript, TypeScript, Python, Rust, JSON, Bash, CSS,
  XML/HTML and SQL; other languages remain readable code. Raw HTML is disabled.
  External images become explicit links instead of tracking requests.
- **openui**: the real `@openuidev/react-lang` 1.0.1 renderer and a versioned
  `hireferry@1` component contract. `/static/chat-openui.json` contains a compact
  component list, example, serialized spec, JSON schema and generated prompt.

The OpenUI contract follows the positional signatures of the supported subset
of [Open Intelligent UI](https://github.com/thesysdev/open-intelligent-ui):
`Col`, `Row`, `Box`, `Grid`, `Card`, `Divider`, `Spacer`, `Title`, `Text`,
`Caption`, `Label`, `Badge`, `Link`, `Markdown`, `Quote`, `Code`, `Math`,
`List`, `ListItem`, `Table`, `Button`, `Input`, `TextArea`, `Select`,
`Checkbox`, `Slider`, `Form`, and `Chart`.

Charts support unstacked bar, line and area, with an accessible data table.
Layout values, rows and source length are bounded. Colors use semantic tokens
or hex literals; arbitrary CSS is not evaluated. Icons, text clamping, and
specialized chart value formatting are not implemented by this subset. Agents
should read the exported contract rather than assuming the demo's complete API.
The chat API accepts at most 10,000 Unicode characters per message; the renderer's
separate 65,536-code-unit fallback guard is not an expanded API quota.

The demo's maps, video, arbitrary HTML/JavaScript, custom functions, other chart
types and automatic tools are not supported. No `toolProvider` is connected.
`@ToAssistant` prepares a draft; `@Set` and `@Reset` change local state. `@OpenUrl`
actions are ignored; use a visible `Link` for a user-clicked URL instead.
Unrecognized/malformed UI shows a readable source fallback. All controls are
disabled during a stream.

OpenUI observability is disabled per Renderer and its development tools are
removed by the production build. The implementation adds no hosted telemetry,
model provider, API key, iframe, or model-generated script execution.

The JavaScript payload is lazy-loaded (about 337 KiB gzip), plus local CSS and
KaTeX WOFF2 fonts fetched as needed. Complete bundled dependency notices are in
`static/chat-renderer-NOTICES.txt`.
