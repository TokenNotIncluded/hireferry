import React, { Component, useCallback, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Renderer } from '@openuidev/react-lang';
import { library } from './library.jsx';
import { Markdown } from './markdown.jsx';
import { draftAction, MAX_SOURCE_LENGTH } from './security.js';

export const capabilities = Object.freeze({ version: 1, formats: ['plain', 'markdown', 'openui'], library: 'hireferry@1', sourceLimit: MAX_SOURCE_LENGTH, actions: ['draft'], specUrl: '/static/chat-openui.json' });

class RenderBoundary extends Component {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidUpdate(previous) {
    if (this.state.failed && previous.body !== this.props.body) this.setState({ failed: false });
  }
  render() {
    return this.state.failed ? <div className="hfy-rich-fallback"><p role="status">此消息无法渲染，已显示原始内容。</p><pre>{this.props.body.slice(0, MAX_SOURCE_LENGTH)}</pre></div> : this.props.children;
  }
}

function OpenUiBody({ body, status, onAction }) {
  const [errors, setErrors] = useState('');
  const onError = useCallback((list) => {
    const next = list.filter((error) => error.severity !== 'warning').slice(0, 4).map((error) => String(error.code)).join(', ');
    setErrors((previous) => previous === next ? previous : next);
  }, []);
  return <>
    <Renderer response={body} library={library} isStreaming={status === 'streaming'} onAction={onAction} onError={onError} toolProvider={null} publishObservability={false} />
    {errors && status !== 'streaming' && <details className="hfy-rich-fallback"><summary>部分交互内容无法显示，查看原文</summary><p>{errors}</p><pre>{body}</pre></details>}
  </>;
}

function Message({ body, format, status, onAction }) {
  if (body.length > MAX_SOURCE_LENGTH) return <div className="hfy-rich-fallback"><p>消息超出富文本显示长度，已显示原文预览。</p><pre>{body.slice(0, MAX_SOURCE_LENGTH)}</pre></div>;
  return <RenderBoundary body={body}><div className="hfy-rich-message" data-format={format} aria-busy={status === 'streaming'}>{format === 'openui' ? <OpenUiBody body={body} status={status} onAction={onAction} /> : format === 'markdown' ? <Markdown text={body} /> : <div className="hfy-rich-plain">{body}</div>}</div></RenderBoundary>;
}

const mounted = new WeakMap();

/**
 * Own one message body element. `update` merges partial props, batches streaming
 * chunks into one frame, and preserves the React root and interactive field state.
 * The caller must call `unmount` before removing/replacing the body element.
 */
export function mount(element, initial = {}) {
  if (!element || element.nodeType !== 1) throw new TypeError('A message body element is required');
  const existing = mounted.get(element);
  if (existing) { existing.update(initial); return existing; }
  const root = createRoot(element);
  let props = { body: '', format: 'markdown', status: 'complete', ...initial };
  let disposed = false;
  let pending = null;
  const schedule = globalThis.requestAnimationFrame?.bind(globalThis) || ((fn) => setTimeout(fn, 0));
  const cancel = globalThis.cancelAnimationFrame?.bind(globalThis) || clearTimeout;
  const onAction = (raw) => {
    if (disposed || props.status === 'streaming') return;
    const detail = draftAction(raw);
    if (!detail) return;
    element.dispatchEvent(new CustomEvent('hireferry:chat-action', { bubbles: true, detail }));
    if (typeof props.onAction === 'function') props.onAction(detail);
  };
  const render = () => {
    pending = null;
    if (disposed) return;
    root.render(<Message body={String(props.body ?? '')} format={props.format} status={props.status} onAction={onAction} />);
  };
  const handle = {
    update(next) {
      if (disposed) return;
      props = { ...props, ...next };
      if (pending === null) pending = schedule(render);
    },
    unmount() {
      if (disposed) return;
      disposed = true;
      if (pending !== null) cancel(pending);
      root.unmount();
      mounted.delete(element);
    },
  };
  mounted.set(element, handle);
  render();
  return handle;
}

if (typeof window !== 'undefined') window.HireFerryChatRenderer = Object.freeze({ mount, capabilities });
