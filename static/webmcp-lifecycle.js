/* Each owner releases only its own tools. Chrome's current API unregisters
 * through AbortSignal; earlier implementations exposed unregisterTool(). */
const sessionChanged = () => ({ content: [{ type: 'text', text: 'Session changed. Reopen the current workspace.' }], isError: true });

export function createWebMCPOwner({
  getContext = () => document.modelContext || navigator.modelContext,
  onError = ({ operation, name, error }) => console.warn(`WebMCP ${operation} failed${name ? `: ${name}` : ''}`, error),
} = {}) {
  let generation = 0, desired = null, queue = Promise.resolve();
  const records = new Set();
  const report = (operation, name, error) => { try { onError({ operation, name, error }); } catch { /* Diagnostics must not break navigation. */ } };
  const current = target => !!target && desired === target && target.generation === generation && target.isCurrent();
  const invalidate = () => {
    for (const record of records) { record.active = false;record.controller.abort(); }
  };
  const release = async record => {
    record.active = false;record.controller.abort();
    if (record.registered && typeof record.context.unregisterTool === 'function') {
      try { await record.context.unregisterTool(record.name); }
      catch (error) {
        if (error?.name !== 'NotFoundError') { report('unregister', record.name, error);return false; }
      }
    }
    records.delete(record);
    return true;
  };
  const enqueue = task => {
    queue = queue.then(task).catch(error => report('lifecycle', '', error));
    return queue;
  };
  const clear = () => {
    generation++;desired = null;invalidate();
    return enqueue(async () => { for (const record of [...records]) await release(record); });
  };
  const sync = ({ scope, tools, isCurrent = () => true }) => {
    const context = getContext();
    if (!context?.registerTool || !tools.length) return clear();
    const signature = JSON.stringify(tools.map(({ execute, ...definition }) => definition));
    // Language and route renders rebuild closures, but do not change ownership
    // or schemas. Preserve existing registrations and in-flight executions.
    if (desired?.scope === scope && desired.context === context && desired.signature === signature && current(desired)) return queue;
    invalidate();
    const target = { scope, tools, context, signature, isCurrent, generation: ++generation };
    desired = target;
    return enqueue(async () => {
      for (const record of [...records]) await release(record);
      if (!current(target)) return;
      const names = new Set();
      for (const definition of tools) {
        if (!current(target)) break;
        if (names.has(definition.name)) { report('register', definition.name, new Error('Duplicate name within one tool owner'));continue; }
        names.add(definition.name);
        // Keep a failed legacy removal tracked; never overwrite or forget it.
        if ([...records].some(record => record.context === context && record.name === definition.name)) continue;
        const record = { name: definition.name, context, controller: new AbortController(), active: true, registered: false };
        records.add(record);
        try {
          await context.registerTool({
            ...definition,
            execute: async (...args) => {
              if (!record.active || !current(target)) return sessionChanged();
              const value = await definition.execute(...args);
              return record.active && current(target) ? value : sessionChanged();
            },
          }, { signal: record.controller.signal });
          record.registered = true;
          if (!current(target)) await release(record);
        } catch (error) {
          records.delete(record);
          if (!record.controller.signal.aborted) report('register', definition.name, error);
        }
      }
    });
  };
  return { sync, clear, isCurrent: scope => desired?.scope === scope && current(desired), idle: () => queue };
}
