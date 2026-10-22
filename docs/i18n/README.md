# README 翻译 / README translations

[简体中文](../../README.md) · [English](README.en.md) · [繁體中文](README.zh-TW.md) · [文档目录](../README.md)

## 中文

根目录的 [README.md](../../README.md) 是简体中文源文档。三种语言的概览使用相同章节、命令和功能范围，读者通过页首链接手动选择语言。专题文档目前以简体中文为主。README 的语言数量与网页界面的语言数量分别说明。

### 同步与新增语言

1. 修改根 README 时，在同一提交中同步英文与繁体中文。命令、配置键、路径、版本号和许可证标识保持一致。
2. 新语言放在本目录的 `README.<语言代码>.md`，例如 `README.ja.md`。保留章节顺序和显式锚点，如 `features`、`quick-start`。
3. 更新所有 README 的语言入口、本页目录和[文档目录](../README.md)中的语言列表。当前语言用粗体，其他语言使用链接。根文档链接为 `../../README.md`，专题文档为 `../<文件名>.md`。
4. 共用 [SVG 配图](../assets/readme/)。图中统一使用简短英文标签，翻译各版本的 `alt` 文本。图片路径为 `../assets/readme/<文件名>.svg`。
5. 检查相对链接、页内导航、代码块和图片显示。CI、发布和版本徽章保留真实来源，不添加未经验证的翻译完成率或测试状态。

## English

The root [README.md](../../README.md) is the Simplified Chinese source. All three overview pages share the same sections, commands and feature scope. Readers choose a language through the links at the top. Detailed guides are currently mainly in Simplified Chinese. Describe README languages and website languages separately.

### Keep translations in sync and add languages

1. Update the English and Traditional Chinese versions in the same commit as the root README. Keep commands, configuration keys, paths, version numbers and license identifiers unchanged.
2. Add a language as `README.<language-code>.md` in this directory, for example `README.ja.md`. Preserve the section order and explicit anchors such as `features` and `quick-start`.
3. Update the language links in every README, the index on this page and the language list in the [documentation index](../README.md). Show the current language in bold and link to the others. Use `../../README.md` for the root and `../<filename>.md` for detailed guides.
4. Reuse the [SVG illustrations](../assets/readme/). Their labels use short English phrases. Translate the `alt` text in each README and use `../assets/readme/<filename>.svg` for image paths.
5. Check relative links, in-page navigation, code blocks and images. Keep CI, release and version badges tied to their real sources. Do not add unverified translation-completion or test-status badges.
