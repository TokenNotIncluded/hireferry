import { createWebMCPOwner } from './webmcp-lifecycle.js';

/* Browser AI and remote MCP use the same bounded schemas and service functions.
 * This catalog has its own lifetime; it never removes the app's other tools. */
const names = new Set(['hf_view_open', 'hf_view_patch', 'hf_graph_query', 'hf_graph_read', 'hf_view_save', 'hf_view_list']);

export function createRelationTools({
  api,
  getContext = () => document.modelContext || navigator.modelContext,
  onError = ({ operation, name, error }) => console.warn(`Relation WebMCP ${operation} failed${name ? `: ${name}` : ''}`, error),
}) {
  const owner = createWebMCPOwner({ getContext, onError });
  let active = null;
  const current = target => !!target && active === target && target.isCurrent();
  const clear = () => { active = null;return owner.clear(); };
  const tools = catalog => (catalog.tools || []).filter(definition => names.has(definition.name)).map(definition => ({
    name: definition.name,
    description: definition.description,
    inputSchema: definition.inputSchema,
    annotations: {
      readOnlyHint: !!definition.annotations?.readOnlyHint,
      consequentialHint: !!definition.annotations?.destructiveHint,
    },
    execute: async (input = {}) => {
      try {
        const value = await api(`/api/admin/relations/tools/${encodeURIComponent(definition.name)}`, { method: 'POST', body: input });
        return { content: [{ type: 'text', text: JSON.stringify(value) }] };
      } catch (error) {
        return { content: [{ type: 'text', text: error.message }], isError: true };
      }
    },
  }));
  const install = target => owner.sync({ scope: target.scope, tools: tools(target.catalog), isCurrent: () => current(target) });
  const sync = ({ scope, isCurrent = () => true }) => {
    if (!getContext()?.registerTool || !isCurrent()) return clear();
    if (active?.scope !== scope || !current(active)) {
      void clear();
      active = { scope, isCurrent, catalog: null, pending: null };
    }
    const target = active;
    if (target.catalog) return install(target);
    // Several renders may finish while discovery is pending. Share one read;
    // a response from an old identity or departed route can never register.
    if (target.pending) return target.pending;
    target.pending = Promise.resolve().then(() => api('/api/admin/relations/tools', { fresh: true })).then(async catalog => {
      if (!current(target)) return;
      target.catalog = catalog;
      await install(target);
    }).catch(error => {
      if (!current(target)) return;
      void owner.clear();
      try { onError({ operation: 'discover', name: '', error }); } catch { /* Diagnostics must not break navigation. */ }
    }).finally(() => { target.pending = null; });
    return target.pending;
  };
  return { sync, clear };
}
