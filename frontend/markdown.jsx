import React, { memo, useMemo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import hljs from 'highlight.js/lib/core';
import javascript from 'highlight.js/lib/languages/javascript';
import typescript from 'highlight.js/lib/languages/typescript';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import json from 'highlight.js/lib/languages/json';
import bash from 'highlight.js/lib/languages/bash';
import css from 'highlight.js/lib/languages/css';
import xml from 'highlight.js/lib/languages/xml';
import sql from 'highlight.js/lib/languages/sql';
import { safeImage, safeLink } from './security.js';

for (const [name, language] of Object.entries({ javascript, typescript, python, rust, json, bash, css, xml, sql })) hljs.registerLanguage(name, language);
const remarkPlugins = [remarkGfm, remarkMath];
const rehypePlugins = [[rehypeKatex, { trust: false, strict: 'ignore', throwOnError: false, maxExpand: 200, maxSize: 20 }]];

export const Code = memo(function Code({ text, language = '', block = true }) {
  const highlighted = useMemo(() => {
    if (!block || text.length > 24000 || !hljs.getLanguage(language)) return null;
    // highlight.js escapes the source before emitting its own span-only markup.
    try { return hljs.highlight(text, { language, ignoreIllegals: true }).value; } catch { return null; }
  }, [text, language, block]);
  const code = highlighted === null ? <code>{text}</code> : <code className="hljs" dangerouslySetInnerHTML={{ __html: highlighted }} />;
  return block ? <pre className="hfy-rich-code" tabIndex={0} aria-label={language ? `${language} code` : 'Code'}>{code}</pre> : code;
});

const components = {
  a({ href, children }) {
    const safe = safeLink(href);
    return safe ? <a href={safe} target="_blank" rel="noopener noreferrer">{children}</a> : <span>{children}</span>;
  },
  img({ src, alt }) {
    const safe = safeImage(src);
    if (safe) return <img src={safe} alt={alt || ''} loading="lazy" referrerPolicy="no-referrer" />;
    const link = safeLink(src);
    return link ? <a href={link} target="_blank" rel="noopener noreferrer">{alt || '查看图片'}</a> : <span>{alt || ''}</span>;
  },
  table({ children }) { return <div className="hfy-rich-table" tabIndex={0}><table>{children}</table></div>; },
  pre({ children }) { return <>{children}</>; },
  code({ className, children }) {
    const text = String(children || '');
    const language = /\blanguage-([\w-]+)/u.exec(className || '')?.[1] || '';
    return <Code text={text} language={language} block={!!language || text.includes('\n')} />;
  },
};

export const Markdown = memo(function Markdown({ text }) {
  return <div className="hfy-rich-markdown"><ReactMarkdown skipHtml remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins} components={components} urlTransform={(url, key) => key === 'src' ? safeLink(url) || safeImage(url) || '' : safeLink(url) || ''}>{text}</ReactMarkdown></div>;
});

export const InlineMarkdown = memo(function InlineMarkdown({ text }) {
  return <ReactMarkdown skipHtml unwrapDisallowed allowedElements={['strong', 'em', 'del', 'code', 'a', 'br']} remarkPlugins={[remarkGfm]} components={components} urlTransform={(url) => safeLink(url) || ''}>{text}</ReactMarkdown>;
});
