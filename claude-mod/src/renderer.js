const TOKEN = /\b(?:PERSON|EMAIL|PHONE|ADDRESS|ORGANIZATION|ID|DATE|LOCATION)_[A-Z0-9_-]+\b/g;

function isLoopback(hostname) {
  return hostname === 'localhost' || hostname === '::1' || /^127(?:\.\d{1,3}){3}$/.test(hostname);
}

/** Ephemeral, in-memory projection capability. No network calls are made here. */
export function createProjectionSession({ sessionId, origin, expiresAt, verifyEvent }) {
  const expectedOrigin = new URL(origin);
  if (expectedOrigin.protocol !== 'http:' || !isLoopback(expectedOrigin.hostname)) {
    throw new TypeError('Projection origin must be HTTP loopback');
  }
  if (typeof sessionId !== 'string' || !sessionId || typeof verifyEvent !== 'function') {
    throw new TypeError('A session id and event authenticator are required');
  }
  // Do not rely on callers to parse the gateway's RFC-3339 wire value: a raw
  // string coerces to NaN, `now >= NaN` is always false, and a stale mapping
  // table would live for the life of the page. Fail closed at construction.
  if (typeof expiresAt !== 'number' || !Number.isFinite(expiresAt) || expiresAt <= Date.now()) {
    throw new TypeError('Projection expiry must be a finite timestamp in the future');
  }
  const mappings = new Map();
  let active = true;

  return Object.freeze({
    acceptMapEvent(event, now = Date.now()) {
      if (!active || now >= expiresAt || !event || event.type !== 'Mapping') return false;
      let eventOrigin;
      try { eventOrigin = new URL(event.origin); } catch { return false; }
      if (eventOrigin.origin !== expectedOrigin.origin || !isLoopback(eventOrigin.hostname) ||
          event.sessionId !== sessionId || typeof event.token !== 'string' ||
          !TOKEN.test(event.token) || typeof event.original !== 'string' ||
          !verifyEvent(event)) return false;
      TOKEN.lastIndex = 0;
      mappings.set(event.token, event.original);
      return true;
    },
    project(text, now = Date.now()) {
      if (!active || now >= expiresAt || typeof text !== 'string') return [{ text: String(text ?? ''), revealed: false }];
      TOKEN.lastIndex = 0;
      const parts = [];
      let cursor = 0;
      for (const match of text.matchAll(TOKEN)) {
        if (match.index > cursor) parts.push({ text: text.slice(cursor, match.index), revealed: false });
        const original = mappings.get(match[0]);
        parts.push({ text: original ?? match[0], revealed: original !== undefined });
        cursor = match.index + match[0].length;
      }
      if (cursor < text.length) parts.push({ text: text.slice(cursor), revealed: false });
      return parts;
    },
    revoke() { active = false; mappings.clear(); }
  });
}

export function installRenderer(on, bridge) {
  on('ui.render', { component: 'AssistantMessage' }, async ($, event, next) => {
    const text = event?.props?.text;
    if (typeof text !== 'string' || !bridge || typeof bridge.project !== 'function') return next(event);
    const parts = bridge.project(text);
    if (!parts.some(part => part.revealed)) return next(event);
    const { Text } = $.ui.resolve(event);
    const children = parts.map(part => part.revealed
      ? Text({ color: 'gray', children: [part.text] })
      : part.text);
    return Text({ children });
  });
}
