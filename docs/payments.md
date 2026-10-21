# Payments / 支付

HireFerry uses hosted, one-time checkout. The website does not receive card numbers.
All local money is stored as integer CNY cents. The browser and MCP client cannot
choose the amount charged for an order.

## Charges

| Action | Charge | Example |
| --- | --- | --- |
| Activate a recruiter account | ¥100 once | ¥100 |
| Publish a job | 10% of monthly base salary | ¥3,000 salary → ¥300 |
| Edit a published job's description | 1% of the original monthly base salary, per edit | ¥3,000 salary → ¥30 |

Job title and base salary are fixed after publication. There are at most three
paid edits. An edit becomes public only after its payment is verified. Job seekers
pay nothing. Configure any allowed rates in TOML; existing orders keep their
original amount.

中文：修改费按原岗位月薪底薪的 1% 收取，不是发布费的 1%。标题与底薪不可修改。
每个岗位最多完成三次付费修改。支付返回页不能将订单改为已支付。

## Stripe

Set the Stripe secret key, webhook secret, and `live_mode` in the TOML file.
Use test keys with `live_mode = false`; live keys require `true`. Restricted keys
are accepted if they have the necessary Checkout permission. The application
fails closed when keys are absent or the configured environment does not match.

Create a **snapshot** webhook destination for:

- `checkout.session.completed`
- `checkout.session.async_payment_succeeded`

Webhook URL: `https://YOUR-HOST/api/webhooks/stripe`.

Checkout uses `/v1/checkout/sessions`, `mode=payment`, CNY minor units, a stable
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
- [Webhook signature verification](https://docs.stripe.com/webhooks?verify=verify-manually)
- [Fulfill orders](https://docs.stripe.com/checkout/fulfillment?payment-ui=stripe-hosted)
- [Idempotent requests](https://docs.stripe.com/api/idempotent_requests)

## Waffo Pancake

Configure merchant ID, store ID, one-time product ID, private RSA key, matching
webhook public RSA key, tax category, and `mode` (`test` or `prod`) in TOML.
The API base URL is `https://api.waffo.ai`. Private keys remain on the server.
Use the actual tax category approved for your business. Recruitment services
must not be described as a different product simply to make checkout work.

The product must support CNY. The application creates a session using:

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

Configure the `order.completed` webhook at
`https://YOUR-HOST/api/webhooks/waffo`. The `X-Waffo-Signature` header contains a
**millisecond** timestamp and a Base64 RSA-SHA256 signature over
`timestamp + "." + exact raw request body`. Verification uses a **45-minute**
window because legitimate retries keep the original signature. Event/payment
deduplication, rather than a narrow time window, prevents duplicate fulfillment.

The handler checks the store, environment, successful payment state, CNY charge,
and local order reference. It uses `chargedAmount` (or the documented legacy
`amount` field), never `listPrice.total`. Waffo does not include the checkout
session ID in this event; the authenticated merchant order reference identifies
the local order instead.

There is no Waffo `cancelUrl`. The buyer returns from its success page by clicking
Done. Abandoning the hosted cashier does not prove either failure or success;
the website keeps its own order state and waits for the verified callback.

Official sources checked on 2026-10-22:

- [Authentication and request signature](https://docs.waffo.ai/api-reference/authentication)
- [Create Checkout Session](https://docs.waffo.ai/api-reference/endpoints/orders/create-checkout-session)
- [Webhook payload, environment, and signature](https://docs.waffo.ai/api-reference/webhooks)
- [API format and idempotency](https://docs.waffo.ai/api-reference/introduction)

## Deployment and limits

- A real HTTPS public URL and correctly configured webhook destinations are
  required. Synchronize the server clock. Keep the TOML file out of Git and
  readable only by the service account.
- Keep the HTTP client's redirects disabled for signed gateway requests.
- The application only activates an order after a valid callback and successful
  database transaction. Browser and MCP operations use the same local orders.
- Unknown callback event types are acknowledged after authentication. This
  module settles successful one-time payments; automatic refunds and dispute
  management require separate explicit workflows. Do not mark a refund complete
  merely because a refund was requested in a payment dashboard.
- Missing merchant credentials do not enable a mock gateway. Local signature
  tests prove parsing and cryptographic behavior, not a live gateway purchase.
- Provider onboarding, accepted business type, CNY availability for the actual
  merchant account, production credentials, and an end-to-end test purchase must
  be confirmed before collecting real payments.

## Focused tests

`cargo test payments::tests` checks fixed-point money parsing, Stripe HMAC
verification, Waffo RSA request signing and callback verification, tampered
payloads, timestamp bounds, the Waffo retry window, environment/store isolation,
unpaid sessions, conflicting order references, and checkout redirect origins.
Commerce integration tests must additionally cover duplicate and concurrent
callbacks, exact local-order amount comparison, blocked users, and the maximum
of three successful description edits.
