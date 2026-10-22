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
currency choices, activation fee, publication and update fee modes, rates, fixed
amounts, update allowance and employer AI salary thresholds come from runtime
configuration. Policy overrides cannot replace these derived values.

The default terms distinguish free, fixed and salary-percentage fees. Their table
lists each currently selectable currency with its publication fee, description
update fee and employer AI base-pay threshold. Amounts are formatted from integer
minor units using the currency's precision, including zero-decimal JPY/KRW; there
is no currency conversion or floating-point money calculation. The table applies
to new roles. Existing jobs and confirmed orders retain their saved currency,
amounts, edit terms and eligibility threshold when configuration changes.

Replacement documents can use these derived variables:

| Variable | Meaning |
| --- | --- |
| `{{public_url}}`, `{{site_name}}`, `{{support_contact}}` | Current site identity and configured support link |
| `{{activation_price}}` | Exact amount and default currency, such as `JPY 235` or `USD 2.35` |
| `{{activation_fee}}` | Localized free-activation or one-time-charge wording |
| `{{publish_fee_rule}}`, `{{edit_fee_rule}}` | Localized wording for the actual fee mode |
| `{{currency_fee_table}}` | Generated Markdown fee and AI-threshold table for all selectable currencies |
| `{{default_currency}}`, `{{available_currencies}}` | Default currency code and currently selectable codes |
| `{{max_edits}}` | Description-update allowance for new roles |
| `{{publish_percent}}`, `{{edit_percent}}` | Exact configured percentages; use only in custom text that accounts for the fee mode |

The legacy percentage variables remain available for existing custom documents,
but do not by themselves describe fixed or free policies. A complete replacement
document is operator-authored text; review its surrounding wording whenever the
fee model changes.

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

The defaults cover the product's AI-native screening and matching workflow through authorized MCP agents,
free headcount changes, one-time restoration, permanent content deletion with
financial references retained, verified Waffo refund events and explicitly
configured Webhooks. Stripe refund records are not synchronized automatically.
These behavior descriptions do not introduce new retention periods or automate
the commitments above. Browser local storage retains language and appearance
preferences.
They do not certify merchant approval or legal compliance. Waffo's business
onboarding separately needs the actual legal operator and its assessment of
whether the paid service is an eligible software product.
