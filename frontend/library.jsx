import React, { useId } from 'react';
import { action, createLibrary, defineComponent, FormNameContext, reactive, tagSchemaId, useFormName, useIsStreaming, useStateField, useTriggerAction } from '@openuidev/react-lang';
import { z } from 'zod/v4';
import { Code as CodeView, InlineMarkdown, Markdown as MarkdownView } from './markdown.jsx';
import { boundedNumber as bound, safeLink } from './security.js';

// Positional signatures deliberately follow the MIT Open Intelligent UI demo for
// the supported subset. These renderers are local and use HireFerry theme tokens.
const node = z.any();
tagSchemaId(node, 'Node');
const children = z.array(node).max(200);
const text = z.string().max(65536);
const align = z.enum(['start', 'center', 'end', 'stretch']);
const justify = z.enum(['start', 'center', 'end', 'between']);
const number = z.number().finite();
const optNumber = number.optional();
const optText = text.optional();
const flex = { start: 'flex-start', end: 'flex-end', center: 'center', stretch: 'stretch', between: 'space-between' };
const gap = (value) => value == null ? undefined : `${bound(value, 3, 0, 24) * 4}px`;
const str = (value) => typeof value === 'string' || typeof value === 'number' ? String(value) : '';
const items = (value, max = 200) => Array.isArray(value) ? value.slice(0, max) : [];
const define = (name, props, description, component) => defineComponent({ name, props: z.object(props), description, component });
const flexStyle = (props) => ({ gap: gap(props.gap), alignItems: flex[props.align], justifyContent: flex[props.justify], flexWrap: props.wrap ? 'wrap' : undefined });
const colorTokens = new Set(['ink', 'muted', 'faint', 'surface', 'info', 'success', 'warning', 'danger', 'purple', 'pink', 'chart1', 'chart2', 'chart3', 'chart4', 'chart5', 'chart6']);
function color(value) {
  if (colorTokens.has(value)) return `var(--hfy-ui-${value})`;
  return typeof value === 'string' && /^#[\da-f]{3,8}$/iu.test(value) ? value : undefined;
}
function size(value) {
  if (typeof value === 'number') return `${bound(value, 0, 0, 1200)}px`;
  return typeof value === 'string' && /^(?:\d{1,3}(?:\.\d{1,2})?(?:%|px|rem)|auto)$/u.test(value) ? value : undefined;
}

const Col = define('Col', { children, gap: optNumber, align: align.optional() }, 'Vertical layout; gap counts 4px steps.', ({ props, renderNode }) => <div className="hfy-ui-col" style={flexStyle(props)}>{renderNode(items(props.children))}</div>);
const Row = define('Row', { children, gap: optNumber, align: align.optional(), justify: justify.optional(), wrap: z.boolean().optional() }, 'Horizontal layout; use wrap for small screens.', ({ props, renderNode }) => <div className="hfy-ui-row" style={flexStyle(props)}>{renderNode(items(props.children))}</div>);
const Box = define('Box', { children: children.optional(), direction: z.enum(['row', 'col']).optional(), gap: optNumber, padding: optNumber, background: optText, radius: z.union([z.enum(['sm', 'md', 'lg', 'xl', 'full']), number]).optional(), border: z.boolean().optional(), width: z.union([text, number]).optional(), height: z.union([text, number]).optional(), flex: optNumber, align: align.optional(), justify: justify.optional(), wrap: z.boolean().optional() }, 'Bounded layout box; use semantic colors.', ({ props, renderNode }) => <div className={`hfy-ui-${props.direction === 'row' ? 'row' : 'col'}`} style={{ ...flexStyle(props), padding: gap(props.padding), background: color(props.background), border: props.border ? '1px solid var(--line)' : undefined, borderRadius: typeof props.radius === 'number' ? bound(props.radius) : ({ sm: 4, md: 8, lg: 12, xl: 16, full: 99 })[props.radius], width: size(props.width), maxHeight: size(props.height), flexGrow: bound(props.flex, 0, 0, 10) }}>{renderNode(items(props.children))}</div>);
const Grid = define('Grid', { children, columns: optNumber, minChildWidth: optNumber, gap: optNumber }, 'Responsive grid with up to 6 columns.', ({ props, renderNode }) => <div className="hfy-ui-grid" style={{ gap: gap(props.gap), gridTemplateColumns: `repeat(auto-fit,minmax(min(100%,${bound(props.minChildWidth, 180, 80, 500)}px),1fr))`, maxWidth: `${bound(props.columns, 2, 1, 6) * bound(props.minChildWidth, 180, 80, 500) + 100}px` }}>{renderNode(items(props.children))}</div>);
const Card = define('Card', { children, variant: z.enum(['outline', 'soft']).optional(), padding: optNumber, gap: optNumber }, 'Panel with border or soft background.', ({ props, renderNode }) => <section className={`hfy-ui-card${props.variant === 'soft' ? ' hfy-ui-soft' : ''}`} style={{ gap: gap(props.gap), padding: gap(props.padding) }}>{renderNode(items(props.children))}</section>);
const Divider = define('Divider', {}, 'Divider.', () => <hr />);
const Spacer = define('Spacer', {}, 'Fills free row space.', () => <span className="hfy-ui-spacer" aria-hidden="true" />);
const Title = define('Title', { text, size: z.enum(['sm', 'md', 'lg', 'xl', '2xl', '3xl']).optional(), color: optText }, 'Heading.', ({ props }) => <div role="heading" aria-level={3} className={`hfy-ui-title hfy-ui-title-${props.size || 'md'}`} style={{ color: color(props.color) }}>{str(props.text)}</div>);
const Text = define('Text', { text: z.union([text, children]), size: z.enum(['xs', 'sm', 'md', 'lg']).optional(), color: optText, weight: z.enum(['normal', 'medium', 'semibold']).optional(), italic: z.boolean().optional(), lineThrough: z.boolean().optional(), tabularNums: z.boolean().optional(), maxLines: optNumber }, 'Inline Markdown text; use Markdown for rich blocks.', ({ props, renderNode }) => <div className={`hfy-ui-text hfy-ui-text-${props.size || 'md'}`} style={{ color: color(props.color), fontWeight: ({ normal: 400, medium: 500, semibold: 600 })[props.weight], fontStyle: props.italic ? 'italic' : undefined, textDecoration: props.lineThrough ? 'line-through' : undefined, fontVariantNumeric: props.tabularNums ? 'tabular-nums' : undefined }}>{Array.isArray(props.text) ? items(props.text).map((part, i) => <React.Fragment key={i}>{typeof part === 'object' && part?.type === 'element' ? renderNode(part) : <InlineMarkdown text={str(part)} />}</React.Fragment>) : <InlineMarkdown text={str(props.text)} />}</div>);
const Caption = define('Caption', { text, color: optText }, 'Secondary text.', ({ props }) => <p className="hfy-ui-caption" style={{ color: color(props.color) }}>{str(props.text)}</p>);
const Label = define('Label', { text }, 'Field label.', ({ props }) => <span className="hfy-ui-label">{str(props.text)}</span>);
const Badge = define('Badge', { text, color: z.enum(['neutral', 'info', 'success', 'warning', 'danger', 'purple', 'pink']).optional(), icon: optText }, 'Status tag; text always conveys the status.', ({ props }) => <span className="hfy-ui-badge" style={{ color: color(props.color) }}>{str(props.text)}</span>);
const Link = define('Link', { text, url: text }, 'Safe http(s) or mailto link, opened only on click.', ({ props }) => { const href = safeLink(props.url); return href ? <a href={href} target="_blank" rel="noopener noreferrer">{str(props.text)}</a> : <span>{str(props.text)}</span>; });
const Markdown = define('Markdown', { text }, 'Markdown with GFM tables, code and KaTeX math. Raw HTML is disabled.', ({ props }) => <MarkdownView text={str(props.text)} />);
const Quote = define('Quote', { text, cite: optText }, 'Quoted text.', ({ props }) => <blockquote>{str(props.text)}{props.cite && <cite>{str(props.cite)}</cite>}</blockquote>);
const Code = define('Code', { code: text, language: optText }, 'Code block with syntax highlighting for common languages.', ({ props }) => <CodeView text={str(props.code)} language={str(props.language)} />);
const MathBlock = define('Math', { tex: text, inline: z.boolean().optional() }, 'KaTeX equation; trusted commands are disabled.', ({ props }) => <MarkdownView text={props.inline ? `$${str(props.tex)}$` : `$$\n${str(props.tex)}\n$$`} />);
const ListItem = define('ListItem', { title: text, description: optText, marker: optText, children: children.optional() }, 'List entry with optional description.', ({ props, renderNode }) => <div><strong>{props.marker && `${str(props.marker)} `}{str(props.title)}</strong>{props.description && <p>{str(props.description)}</p>}{renderNode(items(props.children))}</div>);
const List = define('List', { items: z.array(z.union([text, ListItem.ref])).max(200), marker: z.enum(['bullet', 'number', 'none']).optional(), timeline: z.boolean().optional() }, 'List with optional numbering.', ({ props, renderNode }) => { const Tag = props.marker === 'number' ? 'ol' : 'ul'; return <Tag className={props.marker === 'none' ? 'hfy-ui-list-none' : undefined}>{items(props.items).map((item, i) => <li key={i}>{typeof item === 'string' ? item : renderNode(item)}</li>)}</Tag>; });
const Table = define('Table', { columns: z.array(text).max(30), rows: z.array(z.array(node).max(30)).max(200) }, 'Table. Cells may contain text, numbers or registered components.', ({ props, renderNode }) => <div className="hfy-rich-table" tabIndex={0}><table><thead><tr>{items(props.columns, 30).map((column, i) => <th key={i}>{str(column)}</th>)}</tr></thead><tbody>{items(props.rows).filter(Array.isArray).map((row, r) => <tr key={r}>{items(props.columns, 30).map((_, c) => <td key={c}>{row[c]?.type === 'element' ? renderNode(row[c]) : str(row[c])}</td>)}</tr>)}</tbody></table></div>);

function useField(props, statementId, value = props.value) {
  const id = useId();
  return useStateField(props.label || statementId || id, value);
}
function Field({ label, children }) { return <label className="hfy-ui-field"><span>{label || '输入'}</span>{children}</label>; }
const Button = define('Button', { label: text, action: action().optional(), variant: z.enum(['solid', 'outline', 'soft', 'ghost']).optional(), color: z.enum(['primary', 'danger', 'success']).optional(), size: z.enum(['sm', 'md', 'lg']).optional(), icon: optText, disabled: z.boolean().optional(), block: z.boolean().optional() }, 'User-clicked action. @ToAssistant prepares a draft for review; it never sends automatically.', ({ props }) => { const trigger = useTriggerAction(); const form = useFormName(); const streaming = useIsStreaming(); return <button type="button" className={`hfy-ui-button hfy-ui-button-${props.variant || 'solid'}`} disabled={!!props.disabled || streaming} onClick={() => { void trigger(str(props.label), form, props.action).catch(() => {}); }}>{str(props.label)}</button>; });
const Input = define('Input', { value: reactive(z.union([text, number])), label: optText, type: z.enum(['text', 'number']).optional(), placeholder: optText }, 'Text or number field. Bind value to a $variable.', ({ props, statementId }) => { const field = useField(props, statementId); const disabled = useIsStreaming(); return <Field label={props.label}><input type={props.type === 'number' ? 'number' : 'text'} value={str(field.value)} maxLength={4096} disabled={disabled} placeholder={props.placeholder} onChange={(event) => { const raw = event.target.value; field.setValue(props.type === 'number' && raw !== '' && Number.isFinite(Number(raw)) ? Number(raw) : raw); }} /></Field>; });
const TextArea = define('TextArea', { value: reactive(text), label: optText, placeholder: optText, rows: optNumber }, 'Multiline field.', ({ props, statementId }) => { const field = useField(props, statementId); const disabled = useIsStreaming(); return <Field label={props.label}><textarea value={str(field.value)} maxLength={4096} disabled={disabled} rows={bound(props.rows, 3, 2, 12)} placeholder={props.placeholder} onChange={(event) => field.setValue(event.target.value)} /></Field>; });
const options = z.array(z.union([text, z.object({ value: text, label: text })])).max(100);
const choices = (value) => items(value, 100).filter((option) => typeof option === 'string' || option && typeof option.value === 'string').map((option) => typeof option === 'string' ? { label: option, value: option } : option);
const Select = define('Select', { value: reactive(text), options, label: optText, placeholder: optText }, 'Selection field.', ({ props, statementId }) => { const field = useField(props, statementId); const disabled = useIsStreaming(); return <Field label={props.label}><select value={str(field.value)} disabled={disabled} onChange={(event) => field.setValue(event.target.value)}><option value="">{props.placeholder || '请选择'}</option>{choices(props.options).map((choice, i) => <option key={i} value={choice.value}>{choice.label}</option>)}</select></Field>; });
const Checkbox = define('Checkbox', { checked: reactive(z.boolean()), label: text, lineThrough: z.boolean().optional() }, 'Checkbox.', ({ props, statementId }) => { const field = useField(props, statementId, props.checked); const disabled = useIsStreaming(); return <label className="hfy-ui-checkbox"><input type="checkbox" checked={field.value === true} disabled={disabled} onChange={(event) => field.setValue(event.target.checked)} /><span style={{ textDecoration: field.value === true && props.lineThrough ? 'line-through' : undefined }}>{str(props.label)}</span></label>; });
const Slider = define('Slider', { value: reactive(number), min: number, max: number, step: optNumber, label: optText, unit: optText, marks: z.array(z.object({ value: number, label: text })).optional() }, 'Range slider.', ({ props, statementId }) => { const field = useField(props, statementId); const disabled = useIsStreaming(); const min = bound(props.min, 0, -1e6, 1e6); const max = bound(props.max, min + 100, min + 1, 1e6 + 1); const value = bound(field.value, min, min, max); return <Field label={`${props.label || '数值'}: ${value}${str(props.unit)}`}><input type="range" min={min} max={max} step={bound(props.step, 1, 0.01, 1e6)} value={value} disabled={disabled} onChange={(event) => field.setValue(Number(event.target.value))} /></Field>; });
const Form = define('Form', { children }, 'Groups fields. Submit buttons only prepare a draft.', ({ props, renderNode, statementId }) => { const id = useId(); return <FormNameContext.Provider value={statementId || id}><form className="hfy-ui-form" onSubmit={(event) => event.preventDefault()}>{renderNode(items(props.children))}</form></FormNameContext.Provider>; });

const Chart = define('Chart', { type: z.enum(['bar', 'line', 'area']), data: z.array(z.record(z.string(), z.union([text, number]))).max(100), x: optText, series: z.array(text).max(6).optional(), format: z.enum(['number', 'currency', 'percent']).optional(), currency: optText, stacked: z.literal(false).optional(), height: optNumber }, 'Bar, line or area chart, at most 100 rows and 6 series. Values also available as a table. Stacking is unsupported.', ({ props }) => {
  const rows = items(props.data, 100).filter((row) => row && typeof row === 'object' && !Array.isArray(row));
  if (!rows.length) return <p>暂无图表数据</p>;
  const x = props.x || Object.keys(rows[0]).find((key) => typeof rows[0][key] === 'string') || Object.keys(rows[0])[0];
  const series = items(props.series?.length ? props.series : Object.keys(rows[0]).filter((key) => key !== x && typeof rows[0][key] === 'number'), 6).filter((key) => typeof key === 'string');
  if (!series.length) return <p>暂无数值数据</p>;
  const val = (row, key) => typeof row[key] === 'number' && Number.isFinite(row[key]) ? bound(row[key], 0, -1e12, 1e12) : 0;
  const values = rows.flatMap((row) => series.map((key) => val(row, key)));
  const lo = Math.min(0, ...values), hi = Math.max(1, ...values);
  const px = (i) => 38 + ((i + 0.5) / rows.length) * 540;
  const py = (value) => 190 - (value - lo) / (hi - lo) * 170;
  const label = `${props.type} · ${series.join(', ')} / ${x}`;
  return <figure className="hfy-ui-chart"><svg viewBox="0 0 600 220" role="img" aria-label={label}><line x1="38" x2="580" y1={py(0)} y2={py(0)} stroke="currentColor" opacity="0.3" />{series.map((key, si) => { const paint = `var(--hfy-ui-chart${si + 1})`; const points = rows.map((row, i) => `${px(i)},${py(val(row, key))}`).join(' '); return <g key={key}>{props.type === 'bar' ? rows.map((row, i) => <rect key={i} x={px(i) - 240 / rows.length + si * 480 / rows.length / series.length} y={Math.min(py(0), py(val(row, key)))} width={Math.max(0.5, 480 / rows.length / series.length - 2)} height={Math.abs(py(0) - py(val(row, key)))} fill={paint}><title>{`${str(row[x])}: ${key} ${val(row, key)}`}</title></rect>) : <><polyline points={points} stroke={paint} strokeWidth="2" fill="none" />{props.type === 'area' && <polygon points={`${px(0)},${py(0)} ${points} ${px(rows.length - 1)},${py(0)}`} fill={paint} opacity="0.15" />}</>}</g>; })}{rows.filter((_, i) => i % Math.max(1, Math.ceil(rows.length / 8)) === 0).map((row) => <text key={rows.indexOf(row)} x={px(rows.indexOf(row))} y="210" textAnchor="middle" fill="currentColor" fontSize="10">{str(row[x]).slice(0, 14)}</text>)}</svg><figcaption>{series.join(' · ')}</figcaption><details><summary>查看图表数据</summary><div className="hfy-rich-table"><table><thead><tr>{[x, ...series].map((key) => <th key={key}>{key}</th>)}</tr></thead><tbody>{rows.map((row, i) => <tr key={i}>{[x, ...series].map((key) => <td key={key}>{str(row[key])}</td>)}</tr>)}</tbody></table></div></details></figure>;
});

export const library = createLibrary({ id: 'hireferry@1', root: 'Col', components: [Col, Row, Box, Grid, Card, Divider, Spacer, Title, Text, Caption, Label, Badge, Link, Markdown, Quote, Code, MathBlock, List, ListItem, Table, Button, Input, TextArea, Select, Checkbox, Slider, Form, Chart] });
export const example = 'root = Col([heading, body, table, button])\nheading = Title("面试安排", "lg")\nbody = Markdown("请选择合适的时间。支持 **Markdown**、表格和公式。")\ntable = Table(["时间", "方式"], [["周三 14:00", "视频面试"], ["周四 10:00", "视频面试"]])\nbutton = Button("选择周三 14:00", @ToAssistant("我想选择周三 14:00 面试"))';
export const promptOptions = {
  bindings: true,
  toolCalls: false,
  preamble: 'Render hiring conversation content with HireFerry OpenUI library hireferry@1.',
  additionalRules: [
    'Use only the registered components. This is a compatible subset of thesysdev/open-intelligent-ui, not its complete demo library.',
    'No Query, Mutation, @Run, HtmlView, iframe, arbitrary JavaScript, external media or automatic network actions. Links open only after a user click.',
    '@ToAssistant only prepares a draft; the user reviews and sends it. Use @Set and @Reset for local state.',
    'Use Markdown for rich prose and formulas. Chart supports unstacked bar, line and area; use Table for other data.',
    'The chat API accepts at most 10000 Unicode characters per message. At most 200 list/table rows, 100 chart points, 6 series. Use concise views.',
  ],
  examples: [example],
};
