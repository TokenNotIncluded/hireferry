/* Browser AI and remote MCP share the same bounded schemas and service functions. */
export async function registerRelationTools({ context, api, isCurrent, onRegistered }) {
  let catalog;
  try { catalog = await api('/api/admin/relations/tools', { fresh: true }); }
  catch { return; } // Human controls remain available if browser AI is unsupported.
  if (!isCurrent()) return;
  for (const definition of catalog.tools || []) {
    if (!isCurrent()) return;
    if (!/^hf_(?:view|graph)_[a-z]+$/.test(definition.name)) continue;
    try {
      context.registerTool({
        name: definition.name,
        description: definition.description,
        inputSchema: definition.inputSchema,
        annotations: definition.annotations,
        execute: async (input = {}) => {
          if (!isCurrent()) return { content: [{ type: 'text', text: 'Session changed. Reopen the current workspace.' }], isError: true };
          try {
            const value = await api(`/api/admin/relations/tools/${encodeURIComponent(definition.name)}`, { method: 'POST', body: input });
            if (!isCurrent()) return { content: [{ type: 'text', text: 'Session changed. Reopen the current workspace.' }], isError: true };
            return { content: [{ type: 'text', text: JSON.stringify(value) }] };
          } catch (error) {
            return { content: [{ type: 'text', text: error.message }], isError: true };
          }
        },
      });
      onRegistered(definition.name);
    } catch { /* Older browser implementations can reject an unsupported schema. */ }
  }
}
