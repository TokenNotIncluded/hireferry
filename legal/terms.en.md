# Terms of Service

## Operator and scope {#operator}

Welcome to {{site_name}}, operated by the **{{operator_name}}**  at {{public_url}}. Support, billing, refunds and privacy share one address: {{support_contact}}. These terms cover services provided through the site and its related APIs.

Read these terms and the [Privacy Policy](/privacy) before registering, submitting information or purchasing. You must be at least {{minimum_age}} and authorized when acting for a business or team. Each purchase is governed by the order you confirm. These terms do not waive non-excludable rights under applicable law.

{{operator_identity}}

## What the service provides {#service}

{{site_name}} is an AI-native talent-matching platform. Authorized AI agents connected through MCP automatically organize hiring and job-search needs, screen résumés and match candidates. Connected agent clients run the ongoing tasks. Each side first communicates privately with the platform; relevant profiles and a shared room become accessible after an introduction. Job seekers may separately authorize external outreach.

Services are delivered online through accounts, workspaces, information and conversations, without physical shipment. Employer activation opens account features; job publication and update orders cover their respective role services. Purchases cover online accounts, role management and AI screening and matching software features, without human HR hours or offline recruitment services. We provide automated screening, matching and communication features. We are not a party to employment contracts, do not replace either party’s checks of identities, qualifications or working conditions, and do not guarantee an offer, a hire or a match within any particular time.

## Accounts, information and authorization {#accounts}

Provide accurate résumés, hiring needs and contact details that you are authorized to submit, and keep them current. Do not impersonate others or submit their non-public information without permission. Protect your email, sessions and access tokens, and report suspicious activity.

The platform and authorized AI agents may use information only as needed for matching, service delivery, support and security. External outreach requires separate authorization. You may pause a brief, withdraw outreach permission or revoke an AI token. Withdrawal cannot automatically retrieve copies already received by third parties.

## One-time charges, without renewal {#pricing}

- **Job seekers:**  briefs, platform messages and matching follow-up are free.
- **Employer activation:**  {{activation_fee}} opens employer features for one account, which may manage multiple roles.
- **Job publication:**  the rule for new roles is {{publish_fee_rule}}. Enter fixed base salary only, excluding bonuses, commissions and other projected earnings.
- **Description updates:**  the rule for each update to a new role is {{edit_fee_rule}}, with at most {{max_edits}} updates. Existing roles keep their original fee mode, rates, fixed amounts and allowance. The title, currency and base salary are locked at creation.

{{currency_fee_table}}

The table applies to new roles. Employer AI access also requires an activated account and at least one currently published role meeting its recorded base-pay threshold. Existing roles retain their original threshold; an unpaid draft does not satisfy the publication condition.

There are no membership fees, monthly charges, subscriptions or automatic renewals. Every new charge requires a separate order confirmation. The default currency is {{default_currency}} and the currently selectable currencies are {{available_currencies}}. Changing the currency does not convert amounts. Available payment methods, total payable and applicable taxes are clearly shown at checkout. Free actions take effect on confirmation; paid actions take effect after verified payment. Price changes do not retroactively alter confirmed orders or the terms saved for existing roles.

## Hiring targets, deletion and restoration {#job-management}

Hiring targets can be changed for free as often as needed, without changing base salary, description versions or the description-update allowance. Zero pauses new introductions; a positive number resumes an eligible published role. Existing introduction rooms remain available. Employers maintain the count manually; introductions and recorded hires do not decrement it. Closed roles or roles without publication rights cannot resume hiring by changing the target.

The first deletion moves a role to the trash for free. It can be restored once for free or deleted permanently; deletion after restoration is permanent. Deletion removes the participants' access to the associated profiles and shared room. Eligible introductions regain access after restoration under current permissions, while private platform conversations remain separate. Deletion and restoration use no description update and do not trigger refunds. Permanent deletion clears the title, description and content snapshots in associated orders while retaining necessary order, payment, refund and security references.

## Payment confirmation and delivery {#payment}

Payments through the Waffo channel are processed by **{{payment_provider}}** . We do not store full card numbers or security codes. Paid account access or role changes take effect only after the provider confirms payment and our server verifies the order and notification; free actions need no payment. If you choose Stripe, Stripe handles the hosted checkout. A browser redirect alone is not proof of payment.

Duplicate notifications do not republish a role or consume another update. If paid access is missing, an amount is wrong or an order has not updated, contact {{support_contact}} with the order identifier. We will reconcile it and deliver the service or handle a refund under the rules below. Avoid paying repeatedly to repair the same order.

## Refunds, mistaken payments and disputes {#refunds}

**A full refund may be requested within {{refund_days}} days of payment if the corresponding service has not started.**  Unused activation means no paid role has been submitted and no related AI screening or matching has begun. An unstarted role service means screening, external outreach and introductions for that request have not begun. Merely opening account access or creating a pending order does not mean the service is completed.

Duplicate charges, confirmed billing errors and services we cannot deliver are also refundable. An entirely undeliverable order is refunded in full; for partial delivery, we explain completed work and agree how to handle the undelivered portion. Completed screening, outreach, introductions or role updates are not refundable solely because no hire or offer results, or plans change. Statutory refund rights and other non-excludable rights remain unaffected.

Email {{support_contact}} from your account address with the order identifier, payment time, issue and reason. Do not send full card numbers or security codes. We acknowledge requests within {{refund_ack_business_days}} business days and, after approval, submit the refund to the payment provider within {{refund_processing_business_days}} business days through the original method. Bank or provider settlement may take longer. Refunds are manually reviewed and related entitlements adjusted; we do not promise automatic or immediate settlement.

The site does not initiate refunds. After review, the platform processes them through the payment provider's merchant service. Verified successful Waffo refund notifications update the cumulative refunded amount and order status; failed or unconfirmed notifications do not count as refunded money or reduce recorded net receipts. Partial refunds preserve delivered entitlements. A full activation refund removes the relevant activation, and a full publication refund closes the role. A description-update refund does not roll back text or restore the allowance. Stripe refund status and net amounts are not yet synchronized automatically; contact support to reconcile them. Browser return URLs do not replace authentic notifications and records.

You may contact us first to reconcile a disputed charge. This does not restrict lawful disputes with payment providers, card issuers or authorities.

## AI, content rights and human review {#ai}

The platform uses authorized AI agents to automate briefs, screening, matching and communication, reducing repetitive manual work. Automation requires a connected, running agent. AI output is not a final determination of ability or hiring outcomes. You may request human explanation, review or correction. Employers must independently review candidates, interview and verify conditions, rather than make hiring or rejection decisions solely from AI output.

You retain rights in content you submit and grant only the limited, non-exclusive permission needed to provide the service. We do not use résumés, private messages or hiring briefs to train general-purpose models. AI output can be inaccurate, non-exclusive or lack protectable rights; verify facts, permissions and suitability before use.

AI clients or models you connect may process accessed information under their own policies. Authorize trusted tools and limit sharing. Before introducing additional third-party models to process user information, we will explain the recipient and necessary scope and obtain required authorization. Provider outages can affect AI features; we will explain available alternative communication.

## Webhook notifications you configure {#webhooks}

You can register a receiver and subscribe to events in your workspace. Notifications contain only accessible event and object identifiers, statuses and necessary metadata, without résumés, contact email addresses or message text. Full content requires the corresponding authorization. Notifications can be retried, duplicated, delivered out of order or cancelled after permission is revoked. Use current workspace records as the source of truth.

You can disable or revoke an endpoint. Subsequent deliveries recheck account, introduction and resource permissions. Revocation cannot automatically retrieve copies already received. You choose the receiver and should authorize only trusted recipients.

## Acceptable use {#conduct}

Use the platform only for genuine, lawful job search, hiring and related communication. Do not fabricate jobs or earnings, impersonate others, collect fraudulent job-seeker fees, harass or discriminate, disclose introduced profiles, distribute unlawful or infringing content, or request identity, financial or other sensitive information unrelated to hiring.

Do not use AI or APIs for fraud, phishing, malware, attacks, permission bypass, bulk résumé scraping, impersonation, deepfakes or sexualized content involving minors. Do not train models on others’ information or conversations, resell data or use it for unrelated marketing without explicit consent. Manual and AI actions follow the same permissions and fee rules.

## Suspension, termination and closure {#closure}

You may pause briefs, close roles, revoke AI access or request account closure at {{support_contact}}. Closure creates no new recurring charge. Existing incomplete orders are handled under the delivery and refund rules. Deletion and necessary transaction retention follow the [Privacy Policy](/privacy#retention).

For fraud, unauthorized access, security incidents or serious violations, the platform may restrict features, close roles or suspend accounts, explaining why when practical and compatible with security response. If we stop an undelivered paid service for our own reasons, we refund the undelivered portion. Enforcement does not exclude statutory refund rights.

## Service limits and responsibility {#limits}

We maintain the service with reasonable care but cannot promise uninterrupted networks, third-party sign-in, email, payments or AI, or verify every statement users submit. Known service and billing problems will be investigated. Screening and matching efforts are not a promise of hiring or job-search outcomes.

Where applicable law permits, liability for ordinary service losses is limited to the amount paid for the relevant order giving rise to the claim. This does not limit intentional misconduct, gross negligence, personal-information violations, non-limitable liability or expressly promised refunds. Mandatory applicable law governs responsibility.

## Changes and contact {#changes}

Material changes affecting fees, refunds or major rights are announced at least {{change_notice_days}} days in advance through a site notice or registered email and do not retroactively change confirmed orders. Separate consent will be sought where required. Urgent security or legally required immediate changes will be explained promptly. Chinese and English should express the same rules; translation issues will be reviewed while preserving applicable legal protections.

Operator: {{operator_name}}. Service, billing, refunds and security: {{support_contact}}. See the [Privacy Policy](/privacy) for information use and deletion. Provide account and order details when raising a dispute. These terms do not restrict lawful remedies.
