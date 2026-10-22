export function cliCommands(config, fallbackOrigin) {
  const origin=new URL(config.public_origin||fallbackOrigin).origin;
  const repository=config.cli_distribution?.release_repository;
  if(!repository||! /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository))throw new Error('CLI release source is not configured.');
  return {
    unix:`curl -fsSL ${origin}/install.sh | sh`,
    windows:`irm ${origin}/install.ps1 | iex`,
    oauth:`hfy --server ${origin} auth login --oauth`,
    device:`hfy --server ${origin} auth login --device`,
    identity:`hfy --server ${origin} whoami`,
    capabilities:`hfy --server ${origin} capabilities`,
    skill:`npx skills add ${repository} --skill hireferry`,
    windowsSkill:`npx.cmd skills add ${repository} --skill hireferry`,
    globalSkill:`npx skills add ${repository} --skill hireferry -g`,
    globalWindowsSkill:`npx.cmd skills add ${repository} --skill hireferry -g`,
    source:`cargo install --git https://github.com/${repository} --locked hireferry-cli`,
    releases:`https://github.com/${repository}/releases/latest`,
    docs:`https://github.com/${repository}/blob/main/docs/cli.md`,
  };
}

export function renderCliGuide({config,origin,lang,esc,icon}) {
  const t=(zh,en)=>lang==='en'?en:zh,cmd=cliCommands(config,origin);
  const command=(text,label)=>`<div class="cli-command"><pre><code>${esc(text)}</code></pre><button type="button" class="icon-button" data-action="copy-command" data-copy="${esc(text)}" aria-label="${esc(t('复制','Copy')+' '+label)}" title="${t('复制命令','Copy command')}">${icon('copy')}</button></div>`;
  return `<main id="main" class="page-width cli-guide">
    <section class="cli-hero"><div><h1>HireFerry CLI</h1><p>${t('在终端使用聘渡。','Use HireFerry from your terminal.')}</p></div><div class="cli-hero-links"><a href="#cli-install" class="button">${t('安装 hfy','Install hfy')}${icon('arrow')}</a><a href="${esc(cmd.releases)}" class="text-button" target="_blank" rel="noopener noreferrer">${t('下载安装包','Downloads')} ↗</a></div></section>
    <section class="cli-step" id="cli-install"><h2>${t('安装','Install')}</h2><div class="cli-platforms"><article><h3>Linux / macOS</h3>${command(cmd.unix,'Linux / macOS')}<a href="/install.sh" class="help-link">${t('安装脚本','Installer script')} ↗</a></article><article><h3>Windows PowerShell</h3>${command(cmd.windows,'Windows PowerShell')}<a href="/install.ps1" class="help-link">${t('安装脚本','Installer script')} ↗</a></article></div><p class="cli-inline-note">${t('安装后重新打开终端。运行 <code>hfy --version</code> 检查版本。再次安装即可更新。','Reopen your terminal after installation. Check the version with <code>hfy --version</code>. Run the installer again to update.')}</p><details class="cli-details"><summary>${t('安装选项','Installation options')}</summary><dl class="cli-options"><dt><code>HFY_VERSION</code></dt><dd>${t('指定版本','Pin a version')}</dd><dt><code>HFY_INSTALL_DIR</code></dt><dd>${t('指定安装目录','Choose an install directory')}</dd><dt><code>HFY_NO_PATH=1</code></dt><dd>${t('跳过 PATH 配置','Skip PATH configuration')}</dd></dl><p class="help">${t('在运行安装命令前设置这些环境变量。','Set these environment variables before running the installer.')}</p><p class="help">${t('默认目录：Linux / macOS 为 <code>~/.local/bin</code>。Windows 为 <code>%LOCALAPPDATA%\\HireFerry\\bin</code>。','Default directories: <code>~/.local/bin</code> on Linux / macOS. <code>%LOCALAPPDATA%\\HireFerry\\bin</code> on Windows.')}</p><h3>${t('源码安装 · 需要 Rust','Build from source · Requires Rust')}</h3>${command(cmd.source,t('源码安装','Source installation'))}</details></section>
    <section class="cli-step" id="cli-login"><h2>${t('登录','Sign in')}</h2><div class="cli-platforms"><article><h3>${t('浏览器登录','Browser login')}</h3>${command(cmd.oauth,t('浏览器登录','Browser login'))}<p class="help">${t('在本机浏览器中确认账户。','Confirm your account in this computer’s browser.')}</p></article><article><h3>${t('设备码登录 · SSH / 服务器','Device-code login · SSH / servers')}</h3>${command(cmd.device,t('设备码登录','Device-code login'))}<p class="help">${t('在任意设备打开终端中的验证地址并确认设备码。','Open the displayed URL on any device and confirm the device code.')}</p></article></div><div class="cli-platforms cli-quick-commands"><article><h3>${t('当前身份','Your identity')}</h3>${command(cmd.identity,t('查看身份','Check identity'))}</article><article><h3>${t('可用权限','Permissions')}</h3>${command(cmd.capabilities,t('查看权限','Check permissions'))}</article></div></section>
    <section class="cli-step" id="cli-skill"><details class="cli-details cli-skill-details"><summary>${t('为 AI 助手安装技能','Install the AI skill')}</summary><p>${t('需要 Node.js 22.20 或更新版本。在项目目录运行命令后选择助手。','Requires Node.js 22.20 or later. Run the command in your project directory and select your agent.')}</p><div class="cli-platforms"><article><h3>Linux / macOS</h3>${command(cmd.skill,t('技能安装','Skill installation'))}</article><article><h3>Windows PowerShell</h3>${command(cmd.windowsSkill,t('Windows 技能安装','Windows skill installation'))}</article></div><p class="help">${t('加上 <code>-g</code> 可用于所有项目。安装后重新加载助手。','Add <code>-g</code> to use the skill across projects. Reload your agent after installation.')}</p><p>${t('然后告诉助手：「使用 HireFerry 技能登录并查看我的身份。」','Then ask your agent: “Use the HireFerry skill to sign in and check my identity.”')}</p></details></section>
    <div class="cli-guide-footer"><a href="${esc(cmd.docs)}" target="_blank" rel="noopener noreferrer">${t('使用文档','Documentation')} ↗</a><a href="/oauth/connections">${t('管理连接','Manage connections')}${icon('arrow')}</a></div>
  </main>`;
}
