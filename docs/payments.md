# Payments / 支付

HireFerry uses hosted, one-time checkout. The website does not receive card numbers.
All local money is stored as integer minor units with its recorded ISO currency.
The historical API field suffix `_cents` is retained for compatibility; it does
not mean every currency has two decimal places. The browser and MCP client cannot
choose the amount charged for an order.

Production public URL: `https://hireferry.lmm.best`.
Payment support: `support@lmm.best`. Both values are configured in TOML; the
application does not contain merchant credentials or infer missing product IDs.

## Payment return page / 统一支付返回页

All hosted checkout return URLs use `/billing/return` on the configured HTTPS
`public_url`:

| Gateway return | URL |
| --- | --- |
| Stripe success | `https://hireferry.lmm.best/billing/return?order_id=<local-order-id>&provider=stripe` |
| Stripe cancel | `https://hireferry.lmm.best/billing/return?order_id=<local-order-id>&provider=stripe&canceled=1` |
| Waffo successUrl | `https://hireferry.lmm.best/billing/return?order_id=<local-order-id>&provider=waffo` |

`order_id` is a URL-encoded local order reference. `provider` and `canceled` are
display hints only. Editing, replaying or visiting a return URL never marks an
order paid, cancels an order, grants access or initiates a refund. After login,
the page reads the current user's order through `GET /api/orders/{id}`. A valid
payment may still appear `pending` until its authenticated webhook arrives.
The server can also report `paid`, `paid_needs_review`, `partially_refunded` or
`refunded`; only server state determines the displayed outcome and rights.

## Expired employer activation / 开通订单过期后继续

An unactivated employer can always reach activation from **Orders & payments**,
**My roles**, or the workspace's activation link. The orders page keeps the
current activation action visible; order IDs and older records can be folded.
The payment return page also offers a restart for an expired activation order.

`checkout_expires_at` is a nullable Unix timestamp in seconds. New, unstarted
activation orders expire after 23 hours. Starting checkout records the hosted
session deadline: Stripe's returned `expires_at`, or the first Waffo attempt plus
its requested 1,800-second lifetime. Retrying does not extend that session.
Existing pending activation records receive a conservative deadline during the
idempotent migration. Reading one's orders, starting checkout, or choosing the
employer role marks expired activation orders `expired` and removes their URL.

`POST /api/role` with `role: "employer"`, a provider, and a configured currency
creates a replacement after expiry, or reuses the existing valid activation
order. The transaction serializes concurrent requests. An active account does
not create another activation order. A still-valid order retains its original
amount, currency, and provider; the replacement uses current configured prices
shown for confirmation.

A late verified payment is still recorded. If activation remains unpaid, the
payment activates the account and cancels other pending activation orders. If
another payment already activated it, the additional payment is kept as
`paid_needs_review`. Unresolved received activation funds block a further
activation purchase until reviewed and, when applicable, fully refunded.
Neither an expired URL nor the browser return page is evidence that no money
was charged. After a reported charge, refresh the verified order status first.

This restart path is for employer activation. It does not automatically cancel
or replace role-publication and description-update orders.

## Configurable charges and currency

`[pricing]` selects `currency` and `allowed_currencies`. The default template
keeps CNY for compatibility; the operator can select another supported currency.
New jobs may choose an enabled currency. Recruiter activation defaults to the configured
currency; `pricing.onboarding_amounts` adds explicit fixed prices in other currencies,
independent of job currencies. `POST /api/role` accepts the selected `currency`.
An existing activation order cannot be silently repriced or replaced by another
currency. Existing jobs, orders, refunds and ledgers retain their own
currency after configuration changes. No exchange rate is inferred.

The integration currently implements these charge currencies:

| Gateway | Supported codes and precision |
| --- | --- |
| Stripe, zero decimals | JPY, KRW |
| Stripe, two decimals | AED, ARS, AUD, BRL, CAD, CHF, CNY, COP, CZK, DKK, EUR, GBP, HKD, HUF, IDR, ILS, INR, MXN, MYR, NOK, NZD, PHP, PLN, RON, SEK, SGD, THB, TWD, USD, ZAR |
| Waffo one-time payments | CNY, EUR, GBP, HKD, USD use two decimals; JPY uses zero |

Each gateway has its own `allowed_currencies` configuration. An empty array uses
that integration's implemented set; a nonempty array must be a unique supported
subset and should match the actual merchant account or product. Disabled gateways
do not appear in the public `payment_currencies` map. The browser filters payment
choices by this map; order creation and checkout independently enforce it before
network I/O, including when a cached checkout URL exists. New site currencies
are configured separately in `[pricing]`. Disabling a currency for new checkout
does not relabel historical money or discard valid late payment/refund receipts.

These are charge units. Stripe permits two-decimal HUF/TWD charges even though
its payout rules differ; this application does not initiate payouts. ISK/UGX
compatibility scaling and KWD or other three-decimal charges are not enabled
until their gateway-specific rules are implemented and verified. Unknown codes
fail configuration validation. Enabling a supported code does not prove merchant,
product, country or payment-method availability; verify those externally.

Amounts use integers: USD 12.34 is `1234`; JPY 1234 is `1234`. API job/order data
includes `currency` and `currency_decimals`. Seekers record `salary_currency` with
their salary range. Salary filters use the explicitly selected currency, or infer
the selected job's currency (the site default when no job is selected). Every
numeric filter also requires an exact currency match; numbers from different
currencies are never compared or summed as if they were equivalent.
Revenue is grouped by currency.

`publish_mode` and `edit_mode` each accept `salary_percent`, `fixed` or `free`.
Percentage modes use `publish_bps` / `edit_bps` (10000 = 100%). Fixed modes use
`publish_fixed_minor` / `edit_fixed_minor`. Percentage amounts round half up to
one minor unit; a positive fee that rounds to zero is rejected. Free mode is an
explicit business setting. `onboarding_cents = 0` enables free recruiter
activation. `max_edits` controls the JD revision allowance; headcount changes
and deletion remain free regardless of these settings.

For each additional enabled currency, configure
`[pricing.currency_amounts.<CODE>]` with its own `publish_fixed_minor`,
`edit_fixed_minor` and `employer_ai_min_salary_minor`. These are independent
business prices and qualification thresholds, not currency conversions.
The original job saves its currency, fee mode, edit rates, fixed fee, edit limit
and AI salary threshold. Later configuration changes apply to new jobs only.

Free publishing/editing creates a fulfilled zero-value order with
`provider = "free"`; it never calls a gateway or inserts a cash ledger entry.
The response already contains the applied job. A paid action remains pending
until its signed payment notification arrives.

Default-template examples (these values are configurable):

| Action | Charge | Example |
| --- | --- | --- |
| Activate a recruiter account | ¥100 once | ¥100 |
| Publish a job | 10% of monthly base salary | ¥3,000 salary → ¥300 |
| Edit a published job's description | 1% of the original monthly base salary, per edit | ¥3,000 salary → ¥30 |

Job title, currency and base salary are fixed at creation. The default edit
allowance is three; each job retains the allowance recorded at publication. A paid
revision becomes effective for private platform matching after its payment is
verified; a free revision is applied immediately. Job seekers pay nothing.
Configure any allowed rates in TOML; existing orders keep their original amount.

中文：默认修改费按原岗位月薪底薪的 1% 收取，默认最多三次。费率、固定费用、免费模式、
次数与币种均由配置决定；旧岗位保留创建时的条款。标题、币种与底薪不可修改。
支付返回页不能将订单改为已支付。

## Stripe

Set the Stripe secret key, webhook secret, and `live_mode` in the TOML file.
Use test keys with `live_mode = false`; live keys require `true`. Restricted keys
are accepted if they have the necessary Checkout permission. The application
fails closed when keys are absent or the configured environment does not match.

Create a **snapshot** webhook destination for:

- `checkout.session.completed`
- `checkout.session.async_payment_succeeded`

Webhook URL: `https://hireferry.lmm.best/api/webhooks/stripe`.

Checkout uses `/v1/checkout/sessions`, `mode=payment`, the order’s currency and its minor units, a stable
idempotency key, and the local order reference. Phone collection is disabled.
The callback checks a paid one-time session, currency, amount, environment, and
order reference. The database transaction checks the actual local order before
granting access or publishing content.

The signature is HMAC-SHA256 over `timestamp + "." + exact raw request body`.
The timestamp is in seconds with a five-minute window. Multiple `v1` signatures
are accepted during secret rotation. A successful browser redirect alone does
not count as payment.

Sources checked on 2026-10-22:

- [Create a Checkout Session](https://docs.stripe.com/api/checkout/sessions/create)
- [Stripe currency units and special cases](https://docs.stripe.com/currencies)
- [Webhook signature verification](https://docs.stripe.com/webhooks?verify=verify-manually)
- [Fulfill orders](https://docs.stripe.com/checkout/fulfillment?payment-ui=stripe-hosted)
- [Idempotent requests](https://docs.stripe.com/api/idempotent_requests)

## Waffo Pancake

Configure merchant ID, store ID, one-time product ID, private RSA key, matching
webhook public RSA key, tax category, and `mode` (`test` or `prod`) in TOML.
The API base URL is `https://api.waffo.ai`. Private keys remain on the server.
Use the actual tax category approved for your business. Recruitment services
must not be described as a different product simply to make checkout work.

The configured one-time product must support the currencies allowed for Waffo.
Waffo documents CNY/EUR/GBP/HKD/JPY/USD product prices, with currency-specific
price limits and payment methods; CNY is one-time only and uses WeChat Pay.
Confirm the merchant/product and dynamic `priceSnapshot` limits before enabling
paid service. Product-price limits are not silently assumed to be dynamic-checkout
limits by this application. The application creates a session using:

```text
POST /v1/actions/checkout/create-session
X-Merchant-Id: <merchant ID>
X-Timestamp: <Unix seconds>
X-Signature: <Base64 RSA signature>
X-Idempotency-Key: hireferry-checkout-<local order ID>
```

The request signature uses RSA PKCS#1 v1.5 with SHA-256 over:

```text
POST\n/v1/actions/checkout/create-session\n<TIMESTAMP>\n<BASE64(SHA256(BODY))>
```

`priceSnapshot` contains the exact decimal amount, `taxIncluded = true`, and the
configured tax category. This keeps the advertised charge inclusive of tax.
`orderMerchantExternalId` binds the payment to the local order. The request is
merchant-signed; public Store Slug authentication silently ignores dynamic prices
and is unsuitable here. Each session is valid for 30 minutes.

Configure `order.completed`, `refund.succeeded` and `refund.failed` at
`https://hireferry.lmm.best/api/webhooks/waffo`. The `X-Waffo-Signature` header contains a
**millisecond** timestamp and a Base64 RSA-SHA256 signature over
`timestamp + "." + exact raw request body`. Verification uses a **45-minute**
window because legitimate retries keep the original signature. Event/payment
deduplication, rather than a narrow time window, prevents duplicate fulfillment.

The handler checks the store, environment, successful payment state, recorded currency and charge,
and local order reference. It requires a positive `chargedAmount` and compares
that exact amount with the local order. Waffo's legacy `amount` can fall back to
the list price when the channel did not report a charge, so neither `amount` nor
`listPrice.total` can replace a missing `chargedAmount`. An event without a known
charge is rejected for reconciliation and does not grant service. Waffo does not
include the checkout session ID in this event; the authenticated merchant order
reference identifies the local order instead.

There is no Waffo `cancelUrl`. The buyer returns from its success page by clicking
Done. Abandoning the hosted cashier does not prove either failure or success;
the website keeps its own order state and waits for the verified callback.

Official sources checked on 2026-10-22:

- [Authentication and request signature](https://docs.waffo.ai/api-reference/authentication)
- [Create Checkout Session](https://docs.waffo.ai/api-reference/endpoints/orders/create-checkout-session)
- [Products, currencies, price limits and payment methods](https://docs.waffo.ai/features/products)
- [Webhook payload, environment, and signature](https://docs.waffo.ai/api-reference/webhooks)
- [API format and idempotency](https://docs.waffo.ai/api-reference/introduction)

## Waffo refund notifications / 退款通知

Subscribe the same HTTP webhook destination to all three events:

- `order.completed`
- `refund.succeeded`
- `refund.failed`

Production destination: `POST https://hireferry.lmm.best/api/webhooks/waffo`.
The existing environment-specific RSA public
key, exact-body signature verification, timestamp window, `storeId` and `mode`
checks apply to every event. No additional merchant key is introduced.

Waffo's official refund envelope uses top-level `id` / `eventId` for the refund
ID, `data.paymentId` for the original charge, and `orderMerchantExternalId` plus
`orderMetadata` for the original merchant order. HireFerry requires the two refund
IDs and supplied local references to agree. It records the provider order ID for
audit and checks the payment ID against the already recorded charge.

The amount is `refundedAmount`, with documented legacy `amount` compatibility;
`originalChargedAmount`, when present, must equal the local original payment.
`total` and `originalPayment.total` are not used for cash accounting.
`refund.failed` describes an attempted refund with no cash movement.
[Official refund payload and event semantics](https://docs.waffo.ai/api-reference/webhooks).

### Settlement rules

Each accepted notification is handled inside a SQLite immediate transaction.

| Event / condition | Local result |
| --- | --- |
| Successful refund | Record provider refund ID, order/payment IDs, amount, currency, environment, reason and local timestamps |
| Duplicate successful delivery | No additional deduction, entitlement change or audit event |
| Multiple different successful refunds | Add their amounts; reject a total greater than the recorded original charge |
| Partial refund of a fulfilled order | Set `partially_refunded` and update `orders.refunded_cents`; keep existing service rights |
| Full refund | Set `refunded` and update the corresponding rights below |
| Partial refund of `paid_needs_review` | Keep the review status and update refunded_cents; never turn the unfulfilled charge into a service grant |
| Failed refund | Record the attempted outcome for review; no deduction, order-state change or restoration/revocation of rights |
| Same refund ID first fails, then succeeds | Keep both outcome events; settle the successful outcome once |
| Failure arrives after success | Keep successful state and amounts; record the later failed event without reversing settled money |
| Wrong store, environment, currency, payment, reference or amount | Reject; no partial writes |
| Refund before the original payment notification | Return a retryable non-success response; settle after the original charge has been recorded |
| Unknown local order | Reject; operator should reconcile webhook delivery against the merchant dashboard |

A refund ID cannot later be reused with another order, payment, amount or provider
order ID. Successful totals are derived from immutable payment receipts and
accepted refund records, not client input. An original payment callback repeated
after a refund cannot reopen the order or restore the refunded service.

### Service rights after a full refund

- **Recruiter activation:** remove employer_paid if no other fulfilled, non-fully
  refunded activation order remains. The account and history stay intact.
- **Job publication:** close a currently published job and advance its version.
  No new matching or editing is allowed for that refunded publication. Existing
  introduced conversations keep the normal closed-job history rules.
- **Description edit:** keep the applied text and edit counter. A later refund
  must not roll back newer content or restore an already consumed edit allowance.

Partial refunds retain the original role and publication service; later edits
continue to use the publication's stored pricing terms. Failed refunds do not
change rights. A pending edit whose publication was fully refunded will be held
for manual review if its payment later arrives.

### Storage and revenue

`commerce::migrate` upgrades existing databases with
`orders.refunded_cents INTEGER NOT NULL DEFAULT 0` and creates:

- `payment_refunds`: provider, refund_id, order_id, payment_id,
  provider_order_id, amount_cents, currency, live_mode, status, reason,
  created_at, updated_at, succeeded_at. Unique key:
  (provider, live_mode, refund_id).
- `refund_events`: provider, event_id, event_type, refund_id, order_id,
  payment_id, amount_cents, currency, live_mode, created_at. Unique key:
  (provider, live_mode, event_id, event_type).

The original ledger is preserved. Gross revenue sums recorded charges; successful
refunds subtract from net revenue by their local `succeeded_at` time and payment
environment. Failed refunds never subtract. A period with refunds of earlier
payments can therefore have negative net revenue. The cached order refunded_cents
is updated atomically with successful refund records.

中文：退款必须先由商户在 Waffo 后台或其独立授权流程完成，本应用只接收并验签通知，
没有发起退款 API、自动退款任务或退款按钮。成功退款按实际金额幂等记录，累计不超过
原实收；全额退款撤回对应开通或关闭对应岗位，修改服务退款不回滚 JD 和修改次数。
失败只留记录，既不扣收入也不恢复权限。商户退款、真实到账及回调重试仍需使用实际
商户凭证做端到端验收。

## Deployment and limits

- A real HTTPS public URL and correctly configured webhook destinations are
  required. Synchronize the server clock. Keep the TOML file out of Git and
  readable only by the service account.
- Keep the HTTP client's redirects disabled for signed gateway requests.
- The application only activates an order after a valid callback and successful
  database transaction. Browser and MCP operations use the same local orders.
- Unknown callback event types are acknowledged after authentication. This
  module settles successful one-time payments and Waffo refund outcomes. It does
  not initiate refunds, and Stripe refund notifications/dispute management are not
  implemented. Do not mark a refund complete merely because it was requested in
  a payment dashboard.
- Missing merchant credentials do not enable a mock gateway. Local signature
  tests prove parsing and cryptographic behavior, not a live gateway purchase.
- Provider onboarding, accepted business type, CNY availability for the actual
  merchant account, production credentials, and an end-to-end test purchase must
  be confirmed before collecting real payments.

## Focused tests

`cargo test payments::tests` checks fixed-point money parsing, Stripe HMAC
verification, Waffo RSA request signing and callback verification, tampered
payloads, timestamp bounds, the Waffo retry window, environment/store isolation,
unpaid sessions, conflicting order references, checkout redirect origins, and
canonical billing return URLs with encoded local references and display-only hints.
Commerce integration tests cover duplicate and concurrent payment callbacks,
exact local-order amount comparison, blocked users, and the maximum of three
successful description edits. The added refund tests use locally signed RSA
HTTP events plus real SQLite files: wrong scope/tampering/correlation, early
refund retry, legacy amount parsing, duplicate and reordered outcomes, concurrent
refunds and cumulative ceilings, migration, full publication/activation refunds,
review-state preservation, edit refunds without content rollback, and net revenue
by cash-flow date with test/live isolation. Test
results are recorded separately in the acceptance report; no live refund is
performed by these tests.

## Database upgrade and operating limits

Schema 6 adds job currency and a recorded AI salary threshold. Old rows retain
CNY and the previous CNY 3000 threshold; they are not relabeled with the new site
default. SQLite’s old positive-only order constraint is rebuilt transactionally
to permit zero-value free service orders. Existing orders, payment references,
refunds and amounts are copied unchanged, and foreign-key enforcement is restored.
Take a backup before upgrading.

`[limits]` exposes the previously fixed maximum job headcount, pending-order
count per account, member-list page size and maximum page number.
`session_hours` and `[webhooks]` already configure their corresponding durations
and delivery limits. Protocol signature windows, amount overflow checks and
public-HTTPS destination checks remain fixed validation rules.

## Waffo regions and test mode

`payments.waffo.checkout_regions` controls the UI's Global / China choices.
Each region declares `currencies` and `payment_methods`. These are payment
preferences on the same Waffo API, not separate API origins or a declaration of
billing residency. Waffo collects the actual billing country in its cashier.
The defaults offer card / Apple Pay / Google Pay for USD, EUR, GBP, HKD and JPY
in Global, and WeChat Pay / Alipay for CNY / USD in China. The server signs
`includePaymentMethods`, and rejects region/currency combinations absent from the configuration. New method codes can be enabled in configuration without a code change; the upstream API validates availability.
See [Waffo's current payment matrix](https://docs.waffo.ai/dashboard/payments)
and [checkout parameters](https://docs.waffo.ai/api-reference/endpoints/orders/create-checkout-session).

`POST /api/orders/{id}/checkout` accepts optional `payment_region`. The first
checkout attempt freezes it into the order snapshot; retries reuse it. Currency
and amount are always loaded from the server's order, never from the checkout
request. Paid or already-started activation orders cannot be replaced with a
second currency while an earlier cashier may still receive payment.

For test mode, set `mode = "test"`, `enabled = true`, a **test** API private key,
a product active in **test**, and the platform's **test** webhook public key.
Register a separate webhook with `testMode: true`, the same callback URL, and
`order.completed`, `refund.succeeded`, `refund.failed`. The hosted URL receives
`test=true`; the activation form displays “Test mode · No real charges”. Test
receipts remain `live_mode=false` and are excluded from live revenue. Switching
to production requires the production key, active production product and
production verification key together, after store approval. Preserve the previous
credentials outside Git; never change a test order into a live order.

On 2026-10-22 the live **test API** accepted `includePaymentMethods: ["wechat", "alipay"]` for both CNY and USD, although the published payment matrix still listed only WeChat Pay. This verifies session creation, not a real charge or production approval.
