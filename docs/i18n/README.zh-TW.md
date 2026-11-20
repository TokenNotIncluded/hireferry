<p align="center">
  <img src="../assets/readme/hero.svg" alt="HireFerry 渡船與粒子航線" width="100%">
</p>

<h1 align="center">聘渡 · HireFerry</h1>

<p align="center">AI 原生，自動初篩，讓人才與機會相遇。</p>

<p align="center">
  <a href="../../README.md">简体中文</a> ·
  <a href="README.en.md">English</a> ·
  <strong>繁體中文</strong>
</p>

<p align="center">
  <a href="https://github.com/TokenNotIncluded/hireferry/actions/workflows/ci.yml"><img src="https://github.com/TokenNotIncluded/hireferry/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI 狀態"></a>
  <a href="https://github.com/TokenNotIncluded/hireferry/actions/workflows/cli-release.yml"><img src="https://github.com/TokenNotIncluded/hireferry/actions/workflows/cli-release.yml/badge.svg" alt="CLI 發布狀態"></a>
  <a href="https://github.com/TokenNotIncluded/hireferry/releases/latest"><img src="https://img.shields.io/github/v/release/TokenNotIncluded/hireferry?label=CLI&amp;color=292524" alt="最新 CLI 版本"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-292524" alt="AGPL-3.0-or-later 授權條款"></a>
  <a href="../../Cargo.toml"><img src="https://img.shields.io/badge/Rust-2024-292524?logo=rust" alt="Rust 2024 edition"></a>
  <a href="../self-hosting.md"><img src="https://img.shields.io/badge/SQLite-bundled-57534e?logo=sqlite&amp;logoColor=white" alt="內建 SQLite"></a>
</p>

<p align="center">
  <a href="https://hireferry.lmm.best">官網</a> ·
  <a href="../README.md">文件</a> ·
  <a href="../downloads.md">下載桌面版 / CLI</a> ·
  <a href="https://github.com/TokenNotIncluded/hireferry/issues">問題回報</a>
</p>

<p align="center">
  <a href="https://hireferry.lmm.best/login?role=seeker&amp;utm_source=github&amp;utm_medium=readme&amp;utm_campaign=evergreen&amp;utm_content=seeker">求職者免費註冊</a> ·
  <a href="https://hireferry.lmm.best/login?role=employer&amp;utm_source=github&amp;utm_medium=readme&amp;utm_campaign=evergreen&amp;utm_content=employer">徵才方永久免費發布職缺</a>
</p>

聘渡是一個可自行架設的人才媒合平台。求職者和徵才方分別與平台溝通，由透過 MCP 連接的獲授權 AI agent 自動整理需求、篩選履歷、建立媒合與追蹤進度。持續執行的 agent 客戶端負責自動化循環，伺服器提供權限、業務介面與紀錄。雙方完成媒合後可查看對應資料，並進入獨立對話。

伺服器端使用 **Rust + Axum + SQLite**。單一執行檔內嵌網頁，TOML 管理設定。預設求職與徵才永久免費，網站採捐贈模式；如果聘渡幫你找到工作或招到人，歡迎捐贈支持。

**本頁導覽**：[功能](#features) · [服務流程](#workflow) · [快速開始](#quick-start) · [CLI 與 AI](#cli-and-ai) · [公開目錄](#public-discovery) · [系統架構](#architecture) · [文件](#documentation) · [參與貢獻](#contributing)

<a id="features"></a>

## 功能

| 使用者 | 可以做什麼 |
| --- | --- |
| 求職者 | 管理履歷與求職委託、聯絡平台、查看介紹與外部尋找進度 |
| 徵才方 | 發布職缺、維護徵才人數、管理訂單與優惠、追蹤候選人 |
| HR 團隊 | 依職缺篩選候選人、安排介紹、推進面試與錄用、處理對話與稽核紀錄 |
| AI 助理 | 透過 MCP、HumanGrant 或受支援瀏覽器的 WebMCP 使用獲授權功能 |
| 自行架站者 | 設定幣別與費用、郵件與支付、業務上限、Webhook 和備份 |

網頁提供簡體中文與英文，支援淺色、深色和跟隨系統。README 提供简体中文、English 和繁體中文。

<a id="workflow"></a>

## 服務流程

<img src="../assets/readme/workflow.svg" alt="求職者和徵才方各自與平台 AI 私聊，平台安排介紹後開啟新的媒合對話" width="100%">

1. **提交需求**：求職者填寫資料與委託，徵才方建立職缺。
2. **平台介紹**：HR 依職缺篩選，將適合的候選人介紹給徵才方。
3. **共同推進**：雙方在新對話中溝通，平台追蹤面試與錄用。

資料開放給平台與特定媒合對象。先前與平台的私聊保留在原對話中。對外聯繫由求職者另行授權。

[產品與業務規則 →](../product.md)

<a id="quick-start"></a>

## 快速開始

### 使用已部署的服務

開啟 [hireferry.lmm.best](https://hireferry.lmm.best)，使用 Google 或電子郵件驗證碼登入。選擇求職者或徵才方後進入工作台。

### 本機開發

使用 Rust **1.99.1**，與[目前的 CI](../../.github/workflows/ci.yml) 一致。Linux 需要 C 工具鏈、`pkg-config` 與 OpenSSL 開發套件。

<details>
<summary>Debian / Ubuntu 建置相依套件</summary>

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

開啟 **http://localhost:3000**。開發設定僅監聽本機，電子郵件驗證碼顯示在頁面中。Google、SMTP 和支付閘道依需求設定。

正式部署使用 [config.example.toml](../../config.example.toml)。systemd、Caddy、Docker、管理員初始化與備份請見[自行架設指南](../self-hosting.md)。

## 桌面版下載

Tauri 2 桌面版使用系統 WebView，連接官方或自架的 HireFerry 站點。Windows、macOS 和 Linux 提供 x64 與 ARM64 套件，`hfy` 使用者 CLI 可獨立安裝。Linux 提供 deb、rpm、AppImage 與 Flatpak。

[自動產生的下載矩陣](../downloads.md) · [桌面安裝與發布說明](../desktop.md)。首次完整桌面 Release 前顯示建置目標，發布成功後顯示實際下載連結與檔案大小。

<a id="cli-and-ai"></a>

## CLI 與 AI

### 安裝 `hfy`

Arch Linux 可用 `yay -S hireferry-cli-bin`，開發版使用 `hireferry-cli-git`。自架服務端另行安裝 `hireferry-server-bin` 或 `hireferry-server-git`，詳見 [AUR 指南](../aur.md)。

Linux / macOS：

```sh
curl -fsSL https://hireferry.lmm.best/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://hireferry.lmm.best/install.ps1 | iex
```

安裝程式辨識 x64 / ARM64，驗證 SHA-256 並安裝到使用者目錄。再次執行即可升級。

```sh
hfy auth login
hfy whoami
hfy capabilities --json
```

遠端終端機使用 `hfy auth login --device`。設定、授權範圍、升級與移除請見 [CLI 指南](../cli.md)。

網頁支援快速回饋，目前從原始碼建置的 `hfy` 也可提交。管理員透過 CLI 或 MCP 處理，保留期限與容量可設定。安裝方式與操作請見[回饋指南](../feedback-and-retention.md)。

### 串接 AI

| 方式 | 用途 | 文件 |
| --- | --- | --- |
| MCP | 透過 `/mcp` 使用帳號獲授權的工具 | [API 與 MCP](../api.md) |
| HumanGrant | 由本人選擇權限與期限，AI 使用獨立簽章憑證 | [HumanGrant](../humangrant.md) |
| WebMCP | 在支援該介面的瀏覽器中使用目前工作階段的權限 | [API 與 MCP](../api.md) |
| Agent Skill | 為助理安裝 `hfy` 使用說明 | [CLI 與技能](../cli.md#安装升级与移除-agent-skill) |

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry
```

技能安裝需要 Node.js 22.20+。Windows PowerShell 可用 `npx.cmd`。AI 模型、外部搜尋與對外聯繫工具由營運者另行串接。

<a id="public-discovery"></a>

## 公開目錄與免費 MCP

開啟[公開目錄](https://hireferry.lmm.best/discover)，即可瀏覽仍在招聘的公開職缺，以及主動公開履歷的求職者。私訊、配對歷史、訂單和登入信箱不會進入公開目錄。

需要讓 AI 讀取公開機會時，直接呼叫免費的唯讀端點：

```text
POST https://hireferry.lmm.best/mcp/public
```

不需要登入、API token 或帳號。它提供讀取站點設定、搜尋和讀取公開職缺、搜尋和讀取公開履歷五個工具。支援原生 WebMCP 的瀏覽器開啟 `/discover` 後，也會註冊同一組唯讀工具。完整 JSON-RPC 範例與隱私界線見[公開 MCP 文件](../mcp-public.md)。

<a id="architecture"></a>

## 系統架構

<img src="../assets/readme/architecture.svg" alt="網頁、hfy 和 AI 連接 Rust 與 Axum 服務，服務內嵌網頁資源並使用 SQLite 和 TOML，可選擇串接 Google、SMTP 與支付閘道" width="100%">

| 層次 | 實作 |
| --- | --- |
| 存取入口 | 網頁、`hfy`、MCP、瀏覽器 WebMCP |
| 應用服務 | Rust / Axum、內嵌靜態資源、統一業務權限 |
| 資料與設定 | SQLite、TOML、線上備份 |
| 選用整合 | Google 登入、SMTP、Stripe、Waffo、使用者 Webhook |

正式環境無需 Node.js 服務或獨立資料庫服務。幣別、價格、上限與整合參數在 TOML 中設定。

<a id="documentation"></a>

## 文件

| 想了解什麼 | 從這裡開始 |
| --- | --- |
| 產品流程、隱私與收費 | [產品概覽](../product.md) · [職缺規則](../jobs.md) |
| CLI 安裝與帳號連線 | [CLI 指南](../cli.md) |
| AI 授權與介面 | [身分驗證](../auth.md) · [HumanGrant](../humangrant.md) · [API / MCP](../api.md) |
| 支付、優惠與事件通知 | [支付](../payments.md) · [優惠碼](../promo-codes.md) · [Webhook](../webhooks.md) |
| 快速回饋與資料維護 | [回饋與資料保留](../feedback-and-retention.md) |
| 部署、設定與維護 | [自行架設](../self-hosting.md) · [部署交接](../deployment-handoff.md) |
| 測試結果與後續工作 | [驗收紀錄](../acceptance.md) · [待完成事項](../remaining-work.md) |

[完整文件目錄 →](../README.md) · [简体中文](../../README.md) · [English](README.en.md)

專題文件目前以簡體中文為主。各語言 README 使用相同章節與指令，新增語言請見[翻譯說明](README.md)。

<a id="contributing"></a>

## 參與貢獻

歡迎提交問題、程式碼、文件與翻譯。修改程式碼後執行對應檢查：

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
for suite in tests/*.test.mjs; do node "$suite" || exit 1; done
python3 tests/cli-scripts.test.py
```

前端檢查使用 Node.js 24。完整驗證步驟請見[自行架設指南](../self-hosting.md)，既有紀錄請見[驗收文件](../acceptance.md)。

## 授權條款與致謝

採用 [AGPL-3.0-or-later](../../LICENSE)。

首頁粒子執行環境來自 [particle-sculpture-skill](https://github.com/LIghtJUNction/particle-sculpture-skill)。主題配色使用 [Material Color Utilities](https://github.com/material-foundation/material-color-utilities)，介面視覺參考 [ix.dev](https://ix.dev/)。著作權與相依套件說明請見 [THIRD_PARTY_NOTICES](../../THIRD_PARTY_NOTICES) 和[粒子執行環境聲明](../../static/runtime/NOTICE)。
