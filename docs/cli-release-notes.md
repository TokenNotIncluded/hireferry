Official `hfy` command-line client for HireFerry.

Install, sign in and add the Agent Skill: **https://hireferry.lmm.best/cli**

- Linux and macOS: `curl -fsSL https://hireferry.lmm.best/install.sh | sh`
- Windows PowerShell: `irm https://hireferry.lmm.best/install.ps1 | iex`
- Browser login: `hfy auth login --oauth`
- Device-code login: `hfy auth login --device`
- Multiple saved accounts: `hfy auth accounts`, `hfy auth use <name>`
- One-command account selection: `hfy --account <name> whoami`

Packages cover x64 and ARM64 on Linux, macOS and Windows. Linux binaries use
musl so they can run on both glibc and musl distributions. Every archive contains
the CLI, documentation, license and the HireFerry Agent Skill. `SHA256SUMS` covers
all six archives; the hosted installers verify it before replacing the executable.

No administrator privileges, Rust toolchain or Node.js are required to install
the CLI. The optional `npx skills` installer requires Node.js 22.20 or later.
Login opens a human approval step and does not grant recruitment or administrator
permissions by itself.
Explicit OAuth/device login starts a new approval flow. Separate account aliases
keep credentials isolated, and the selected default is stored in the private CLI
configuration directory.
