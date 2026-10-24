<p align="center">
  <img src="../assets/readme/hero.svg" alt="HireFerry sailboat and particle trail" width="100%">
</p>

<h1 align="center">HireFerry · 聘渡</h1>

<p align="center">AI-native screening and matching to connect people with opportunities.</p>

<p align="center">
  <a href="../../README.md">简体中文</a> ·
  <strong>English</strong> ·
  <a href="README.zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="https://github.com/TokenNotIncluded/hireferry/actions/workflows/ci.yml"><img src="https://github.com/TokenNotIncluded/hireferry/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI status"></a>
  <a href="https://github.com/TokenNotIncluded/hireferry/actions/workflows/cli-release.yml"><img src="https://github.com/TokenNotIncluded/hireferry/actions/workflows/cli-release.yml/badge.svg" alt="CLI release status"></a>
  <a href="https://github.com/TokenNotIncluded/hireferry/releases/latest"><img src="https://img.shields.io/github/v/release/TokenNotIncluded/hireferry?label=CLI&amp;color=292524" alt="Latest CLI release"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-292524" alt="AGPL-3.0-or-later license"></a>
  <a href="../../Cargo.toml"><img src="https://img.shields.io/badge/Rust-2024-292524?logo=rust" alt="Rust 2024 edition"></a>
  <a href="../self-hosting.md"><img src="https://img.shields.io/badge/SQLite-bundled-57534e?logo=sqlite&amp;logoColor=white" alt="Bundled SQLite"></a>
</p>

<p align="center">
  <a href="https://hireferry.lmm.best">Website</a> ·
  <a href="../README.md">Documentation</a> ·
  <a href="../downloads.md">Desktop / CLI downloads</a> ·
  <a href="https://github.com/TokenNotIncluded/hireferry/issues">Report an issue</a>
</p>

HireFerry is a recruitment platform you can host yourself. Job seekers and employers each communicate with the platform AI. Authorized AI agents connected through MCP automate requirement analysis, résumé screening, matching and follow-up. Connected agent clients run the automation loop; the server provides permissions, business tools and records. Once introduced, both parties can view the relevant profile and job and start a separate conversation.

The server uses **Rust + Axum + SQLite**. One binary embeds the website, with configuration in TOML. Job seekers use the service for free. Employer fees follow the site's configuration.

**On this page**: [Features](#features) · [How it works](#workflow) · [Quick start](#quick-start) · [CLI and AI](#cli-and-ai) · [Public discovery](#public-discovery) · [Architecture](#architecture) · [Documentation](#documentation) · [Contributing](#contributing)

<a id="features"></a>

## Features

| Who it is for | What they can do |
| --- | --- |
| Job seekers | Manage resumes and job-search mandates, contact HR, view introductions and external search progress |
| Employers | Publish jobs, manage hiring targets, orders and promotions, follow up with candidates |
| HR teams | Find candidates for a job, make introductions, track interviews and hiring, manage conversations and audit records |
| AI assistants | Use authorized features through MCP, HumanGrant or WebMCP in supported browsers |
| Site operators | Configure currencies and fees, email and payments, business limits, webhooks and backups |

The website supports Simplified Chinese and English, with light, dark and system appearance. The README is available in 简体中文, English and 繁體中文.

<a id="workflow"></a>

## How it works

<img src="../assets/readme/workflow.svg" alt="Job seekers and employers each chat privately with HR. An introduction opens a new shared conversation." width="100%">

1. **Share requirements**: job seekers add their profile and search mandate. Employers create a job.
2. **Meet through HR**: HR finds suitable candidates and introduces them to the employer.
3. **Move forward together**: both parties talk in a new conversation while HR follows interviews and hiring.

Profiles and jobs are visible to the platform and the specific parties introduced. Earlier private HR chats stay in their original conversations. External outreach requires the job seeker's separate consent.

[Product and business rules →](../product.md)

<a id="quick-start"></a>

## Quick start

### Use the hosted service

Open [hireferry.lmm.best](https://hireferry.lmm.best) and sign in with Google or an email code. Choose job seeker or employer to enter your workspace.

### Develop locally

Use Rust **1.99.1**, matching [the current CI](../../.github/workflows/ci.yml). Linux builds need a C toolchain, `pkg-config` and the OpenSSL development package.

<details>
<summary>Debian / Ubuntu build dependencies</summary>

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev ca-certificates
```

</details>

```sh
git clone https://github.com/TokenNotIncluded/hireferry.git
cd hireferry
cp config.development.toml config.toml
chmod 600 config.toml
cargo run --locked -- --config config.toml serve
```

Open **http://localhost:3000**. The development configuration listens only on the local machine and displays email codes on the page. Configure Google, SMTP and payment gateways as needed.

Use [config.example.toml](../../config.example.toml) for production. The [self-hosting guide](../self-hosting.md) covers systemd, Caddy, Docker, administrator setup and backups.

## Desktop downloads

The Tauri 2 desktop client uses the system WebView and connects to the official or a self-hosted HireFerry site. Windows, macOS and Linux packages cover x64 and ARM64; the `hfy` CLI remains independently installable. Linux packaging includes deb, rpm, AppImage and Flatpak.

[Generated download matrix](../downloads.md) · [Desktop installation and release guide](../desktop.md). The matrix shows configured targets until the first complete desktop release, then verified asset links and actual sizes.

<a id="cli-and-ai"></a>

## CLI and AI

### Install `hfy`

On Arch Linux, use `yay -S hireferry-cli-bin`, or `hireferry-cli-git` for main builds. Install the server separately with `hireferry-server-bin` or `hireferry-server-git`. See the [AUR guide](../aur.md).

Linux / macOS:

```sh
curl -fsSL https://hireferry.lmm.best/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://hireferry.lmm.best/install.ps1 | iex
```

The installer detects x64 / ARM64, verifies SHA-256 checksums and installs to your user directory. Run it again to upgrade.

```sh
hfy auth login
hfy whoami
hfy capabilities --json
```

For a remote terminal, use `hfy auth login --device`. See the [CLI guide](../cli.md) for configuration, authorization scopes, upgrades and removal.

Send quick feedback from the website or a build of `hfy` from the current source. Administrators handle it through CLI or MCP, with configurable retention and capacity. See the [feedback guide](../feedback-and-retention.md) for installation and usage.

### Connect AI

| Method | Purpose | Documentation |
| --- | --- | --- |
| MCP | Use the account's authorized tools through `/mcp` | [API and MCP](../api.md) |
| HumanGrant | Let the person choose permissions and expiry. AI receives a separate credential bound to its signing key | [HumanGrant](../humangrant.md) |
| WebMCP | Use the current session's permissions in browsers that support the interface | [API and MCP](../api.md) |
| Agent Skill | Install instructions that help an assistant use `hfy` | [CLI and skills](../cli.md#安装升级与移除-agent-skill) |

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry
```

Skill installation requires Node.js 22.20+. Use `npx.cmd` in Windows PowerShell if needed. Site operators connect AI models, external search and outreach tools separately.

<a id="public-discovery"></a>

## Public discovery and free MCP

Open the [public directory](https://hireferry.lmm.best/discover) to browse published roles that are still hiring and résumés that seekers have explicitly made public. Private messages, match history, orders and sign-in email addresses stay out of the directory.

AI clients can read the same public opportunities through the free, read-only endpoint:

```text
POST https://hireferry.lmm.best/mcp/public
```

No login, API token or account is required. It exposes five tools for site configuration, job search and details, and public résumé search and details. A browser that supports native WebMCP registers the same tools when `/discover` is open. See [public MCP documentation](../mcp-public.md) for JSON-RPC examples and privacy boundaries.

<a id="architecture"></a>

## Architecture

<img src="../assets/readme/architecture.svg" alt="The website, hfy and AI connect to a Rust and Axum service with embedded web assets, SQLite and TOML. Google, SMTP and payment gateways are optional integrations." width="100%">

| Layer | Implementation |
| --- | --- |
| Entry points | Website, `hfy`, MCP, browser WebMCP |
| Application | Rust / Axum, embedded static assets, shared business authorization |
| Data and configuration | SQLite, TOML, online backups |
| Optional integrations | Google sign-in, SMTP, Stripe, Waffo, user webhooks |

Production needs no Node.js service or separate database service. Currencies, prices, limits and integration settings are configured in TOML.

<a id="documentation"></a>

## Documentation

| What you need | Start here |
| --- | --- |
| Product workflow, privacy and pricing | [Product overview](../product.md) · [Job rules](../jobs.md) |
| CLI installation and account connections | [CLI guide](../cli.md) |
| AI authorization and APIs | [Authentication](../auth.md) · [HumanGrant](../humangrant.md) · [API / MCP](../api.md) |
| Payments, promotions and events | [Payments](../payments.md) · [Promotion codes](../promo-codes.md) · [Webhooks](../webhooks.md) |
| Quick feedback and data maintenance | [Feedback and retention](../feedback-and-retention.md) |
| Deployment, configuration and maintenance | [Self-hosting](../self-hosting.md) · [Deployment handoff](../deployment-handoff.md) |
| Test results and next steps | [Acceptance records](../acceptance.md) · [Remaining work](../remaining-work.md) |

[Full documentation index →](../README.md) · [简体中文](../../README.md) · [繁體中文](README.zh-TW.md)

Detailed guides are currently mainly in Simplified Chinese. Each README uses the same sections and commands. See the [translation guide](README.md) to add a language.

<a id="contributing"></a>

## Contributing

Issues, code, documentation and translations are welcome. After changing code, run the relevant checks:

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
for suite in tests/*.test.mjs; do node "$suite" || exit 1; done
python3 tests/cli-scripts.test.py
```

Browser asset checks use Node.js 24. See the [self-hosting guide](../self-hosting.md) for the full validation steps and the [acceptance documentation](../acceptance.md) for existing records.

## License and credits

Licensed under [AGPL-3.0-or-later](../../LICENSE).

The homepage particle runtime comes from [particle-sculpture-skill](https://github.com/LIghtJUNction/particle-sculpture-skill). Theme colors use [Material Color Utilities](https://github.com/material-foundation/material-color-utilities), with visual inspiration from [ix.dev](https://ix.dev/). Copyright and dependency notices are in [THIRD_PARTY_NOTICES](../../THIRD_PARTY_NOTICES) and the [particle runtime notice](../../static/runtime/NOTICE).
