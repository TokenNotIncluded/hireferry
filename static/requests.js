/* Short-lived navigation work stays in memory and is discarded at auth boundaries. */
export class RequestTimeoutError extends Error {
  constructor(method) {
    super('Request timed out');
    this.name = 'RequestTimeoutError';
    this.uncertain = method !== 'GET';
  }
}

const copy = value => typeof structuredClone === 'function' ? structuredClone(value) : JSON.parse(JSON.stringify(value));
const ownWorkspaceReads = new Set(['/api/profile', '/api/mandate', '/api/outreach']);

export function createRequestClient(transport, { now = Date.now, readTimeout = 12000, writeTimeout = 20000, prefetchAge = 3000, maxPrefetch = 6 } = {}) {
  const pending = new Map();
  const prefetched = new Map();
  const controllers = new Map();
  let revision = 0;

  function invalidate({ abort = false } = {}) {
    revision += 1;
    pending.clear();
    prefetched.clear();
    if (abort) for (const controller of controllers.keys()) controller.abort();
  }

  function cancel(group) {
    for (const [path, entry] of pending) if (entry.group === group) pending.delete(path);
    for (const [controller, owner] of controllers) if (owner === group) controller.abort();
  }

  async function request(path, { fresh = false, prefetch = false, timeout, group, ...options } = {}) {
    const method = (options.method || 'GET').toUpperCase();
    const shareable = method === 'GET' && !fresh && !options.signal && !options.headers && !options.body && !options.credentials;
    const canPrefetch = shareable && ownWorkspaceReads.has(path);
    // Only the three own-account screens above may hold a response between hover and click.
    // Conversations, introduced records, permissions, payment state and tokens are never stored.
    if (prefetch && !canPrefetch) return;
    if (shareable) {
      const cached = prefetched.get(path);
      if (cached) {
        if (!prefetch) prefetched.delete(path);
        if (cached.until > now()) return copy(cached.value);
        prefetched.delete(path);
      }
      const existing = pending.get(path);
      if (existing) { if (!prefetch) existing.consumed = true; return copy(await existing.promise); }
    }
    if (fresh) { prefetched.delete(path); pending.delete(path); }

    const controller = new AbortController();
    controllers.set(controller, group);
    const outerSignal = options.signal;
    const abort = () => controller.abort(outerSignal.reason);
    if (outerSignal?.aborted) abort();
    else outerSignal?.addEventListener('abort', abort, { once: true });
    const startedRevision = revision;
    let timer;
    let abortListener;
    const interrupted = new Promise((_, reject) => {
      abortListener = () => reject(controller.signal.reason || new DOMException('Request canceled', 'AbortError'));
      if (controller.signal.aborted) abortListener();
      else controller.signal.addEventListener('abort', abortListener, { once: true });
      timer = setTimeout(() => controller.abort(new RequestTimeoutError(method)), timeout ?? (method === 'GET' ? readTimeout : writeTimeout));
    });
    const entry = { promise: null, consumed: !prefetch, group };
    const operation = Promise.race([
      Promise.resolve().then(() => transport(path, { ...options, method, signal: controller.signal })),
      interrupted,
    ]).then(value => {
      if (prefetch && !entry.consumed && revision === startedRevision && pending.get(path) === entry) {
        prefetched.delete(path);
        prefetched.set(path, { value: copy(value), until: now() + prefetchAge });
        while (prefetched.size > maxPrefetch) prefetched.delete(prefetched.keys().next().value);
      }
      return value;
    }).finally(() => {
      clearTimeout(timer);
      outerSignal?.removeEventListener('abort', abort);
      controller.signal.removeEventListener('abort', abortListener);
      controllers.delete(controller);
      if (pending.get(path) === entry) pending.delete(path);
    });
    entry.promise = operation;
    if (shareable) pending.set(path, entry);
    return copy(await operation);
  }
  return { request, invalidate, cancel };
}

export function createSessionReader(load, { now = Date.now, maxAge = 30000 } = {}) {
  let value;
  let checkedAt = -Infinity;
  let pending = null;
  let revision = 0;
  const fresh = () => value !== undefined && now() - checkedAt < maxAge;
  const invalidate = () => { revision += 1; checkedAt = -Infinity; value = undefined; pending = null; };
  async function read({ force = false } = {}) {
    if (!force && fresh()) return copy(value);
    if (pending) return copy(await pending);
    const startedRevision = revision;
    const operation = Promise.resolve().then(load).then(result => {
      if (revision !== startedRevision) throw new DOMException('Session changed', 'AbortError');
      value = copy(result);
      checkedAt = now();
      return result;
    }).finally(() => { if (pending === operation) pending = null; });
    pending = operation;
    return copy(await operation);
  }
  return { read, invalidate, fresh };
}

export function navigationReads(url, user) {
  if (url.pathname !== '/app' || !user || user.role === 'unassigned') return [];
  if ([...url.searchParams.keys()].some(key => key !== 'tab')) return [];
  if (url.searchParams.get('tab') === 'profile') return ['/api/profile'];
  if (url.searchParams.get('tab') === 'mandate' && user.role === 'seeker') return ['/api/mandate', '/api/outreach'];
  return [];
}
