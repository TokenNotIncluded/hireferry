# Configuring public policies

`/terms` and `/privacy` fetch sanitized content from `/api/legal/terms?lang=zh`
and `/api/legal/privacy?lang=en`. Both `zh` and `en` are supported. These public
responses contain rendered policy text and version metadata, never config paths
or provider credentials.

Policy text is separate from application code:

- `legal/*.md` contains the four default Markdown templates.
- `legal/defaults.json` contains fallback policy values.
- `[legal.values]` in the runtime TOML overrides those values, including the
  operator, refund window, retention periods, version and effective date.
- `[legal.document_files]` optionally selects complete replacement Markdown
  documents. Paths are relative to the config file, or may be absolute. Copy
  the default templates to the config directory to edit them without rebuilding.

See `config.example.toml` for all settings. The website origin, support address,
activation price, publication percentage, update percentage and update count
always come from existing runtime configuration; policy overrides cannot silently
replace those derived values.

Templates use `{{operator_name}}`, `{{refund_days}}` and similar variables.
`{{operator_identity}}` is optional public text for the confirmed legal operator
and country/region. `{{support_contact}}` produces the configured support link.
Unknown variables fail `check-config` instead of appearing in public text.
Variables are plain single-line text; raw HTML and unsafe links in documents are
filtered. Use `## Heading {#refunds}` for stable section links and the automatic
contents list.

After changing values or document files, run:

```sh
hireferry --config /etc/hireferry/config.toml check-config
systemctl restart hireferry
```

Read both public pages back after a change. Record the version, effective date
and reviewed text in Git or the operator's document history. Announce material
changes according to the currently effective policy.

Policy values describe commitments the operator must fulfil. They do not create
automatic refund, erasure, retention, log rotation or notification workflows.
Account closure and refunds currently require manual support handling. Keep
server log retention and actual handling procedures aligned with the published
values. A request to retain necessary transaction records does not authorize
retaining an unrelated complete résumé or private conversation.

The defaults cover the product's current human HR and AI HR matching workflow.
They do not certify merchant approval or legal compliance. Waffo's business
onboarding separately needs the actual legal operator and its assessment of
whether the paid service is an eligible software product.
