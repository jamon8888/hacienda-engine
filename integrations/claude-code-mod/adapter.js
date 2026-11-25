// Authenticated host adapter for the Hacienda Claude Code Mod.
//
// The Mod renderer itself performs no network I/O. This module is loaded by the
// host BEFORE `claude-mod/register.js`; it bootstraps a short-lived, authenticated
// projection session over loopback, injects the bridge into `globalThis`, and
// streams local-only mapping events into the in-memory projection session.
//
// Nothing here writes originals to disk, logs them, or sends them to the
// inference provider. The gateway bearer is used only on loopback Hacienda
// requests and is never placed in a provider-visible message.

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

import { createProjectionSession } from '../../claude-mod/src/renderer.js';

export const CORRELATION_HEADER = 'x-hacienda-projection-correlation';
export const CAPABILITY_HEADER = 'x-hacienda-projection-capability';
const CORRELATION_ENV = 'HACIENDA_PROJECTION_CORRELATION';
const CUSTOM_HEADERS_ENV = 'ANTHROPIC_CUSTOM_HEADERS';
const CORRELATION_FILE = join(homedir(), '.claude', 'hacienda-projection-correlation');
const SETTINGS_FILE = join(homedir(), '.claude', 'settings.json');

// Hacienda pseudonyms are `[CATEGORY:key_id:data]`; the Mod token scanner
// recognises `CATEGORY_KEY_DATA` inside brackets. Keeping the brackets makes
// adjacent tokens independent and lets us restore the exact wire token for
// segments that are not revealed.
const HACIENDA_TOKEN = /\[([A-Z][A-Z0-9_]*):([A-Za-z0-9_-]+):([A-Za-z0-9_-]+)\]/g;
const MOD_TOKEN = /[A-Z][A-Z0-9_]*_[A-Z0-9_-]+/g;

function loopbackOrigin(value) {
  const url = new URL(value);
  if (url.protocol !== 'http:') throw new Error('Hacienda projection origin must be HTTP');
  if (!/^127(?:\.\d{1,3}){3}$/.test(url.hostname)
    && url.hostname !== 'localhost'
    && url.hostname !== '::1') {
    throw new Error('Hacienda projection origin must be loopback');
  }
  if (url.pathname !== '/' || url.search || url.hash) {
    throw new Error('Hacienda projection gateway must be an origin only');
  }
  return url.origin;
}

function readSettings(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch {
    return {};
  }
}

function correlationFromHeaders(value) {
  if (typeof value !== 'string') return undefined;
  for (const line of value.split(/\r?\n/)) {
    const index = line.indexOf(':');
    if (index < 0) continue;
    if (line.slice(0, index).trim().toLowerCase() === 'x-hacienda-projection-correlation') {
      return line.slice(index + 1).trim();
    }
  }
  return undefined;
}

export function resolveCorrelation(options = {}) {
  if (options.correlation) return options.correlation;
  if (options.env?.[CORRELATION_ENV]) return options.env[CORRELATION_ENV];
  try {
    const value = readFileSync(options.correlationFile ?? CORRELATION_FILE, 'utf8').trim();
    if (value) return value;
  } catch {
    // Fall through to settings; no correlation means no projection.
  }
  const settings = options.settings ?? readSettings(options.settingsFile ?? SETTINGS_FILE);
  return correlationFromHeaders(settings?.env?.[CUSTOM_HEADERS_ENV]);
}

export function resolveCredential(options = {}) {
  if (options.credential) return options.credential;
  if (options.env?.ANTHROPIC_AUTH_TOKEN) return options.env.ANTHROPIC_AUTH_TOKEN;
  const settings = options.settings ?? readSettings(options.settingsFile ?? SETTINGS_FILE);
  const helper = options.credentialCommand ?? settings?.apiKeyHelper;
  if (typeof helper === 'string' && helper.trim()) {
    // The installer writes a shell-quoted absolute path. Resolve it directly
    // rather than through a shell so no user setting can inject a command.
    const path = helper.trim().replace(/^['"]|['"]$/g, '');
    try {
      return execFileSync(path, { encoding: 'utf8', timeout: 5000 }).trim();
    } catch {
      return undefined;
    }
  }
  return undefined;
}

function normalizeToken(token) {
  const match = /^\[([A-Z][A-Z0-9_]*):([A-Za-z0-9_-]+):([A-Za-z0-9_-]+)\]$/.exec(token);
  if (!match) return token;
  return `${match[1]}_${match[2].toUpperCase()}_${match[3]}`;
}

function normalizeText(text) {
  const originals = new Map();
  // The NUL sentinel separates adjacent tokens (which the Mod scanner would
  // otherwise merge) and is stripped from every projected segment below.
  const normalized = text.replace(HACIENDA_TOKEN, (wireToken, category, key, data) => {
    const modToken = `${category}_${key.toUpperCase()}_${data}`;
    originals.set(modToken, wireToken);
    return `${modToken}\u0000`;
  });
  return { normalized, originals };
}

function restoreTokens(text, originals) {
  const restored = originals.size === 0
    ? text
    : text.replace(MOD_TOKEN, token => originals.get(token) ?? token);
  return restored.replace(/\u0000/g, '');
}

/** Parse an SSE body into JSON events, tolerating chunk boundaries. */
async function consumeEvents(response, onEvent) {
  const reader = response.body?.getReader?.();
  if (!reader) throw new Error('Projection stream has no body');
  const decoder = new TextDecoder();
  let buffer = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let boundary;
    while ((boundary = buffer.indexOf('\n\n')) >= 0) {
      const frame = buffer.slice(0, boundary);
      buffer = buffer.slice(boundary + 2);
      const data = frame
        .split(/\r?\n/)
        .filter(line => line.startsWith('data:'))
        .map(line => line.slice(5).trimStart())
        .join('\n');
      if (!data) continue;
      try {
        const event = JSON.parse(data);
        if (event && typeof event === 'object') onEvent(event);
      } catch {
        // An unparseable frame is ignored; it can never reveal an original.
      }
    }
  }
}

/**
 * Bootstrap the authenticated Mod projection bridge.
 *
 * Returns an object matching the `haciendaProjectionBridge` contract:
 * `project(text)` → ephemeral `{text, revealed}` segments and `revoke()`.
 * The returned `close()` stops the local stream; it does not alter Claude's
 * stored transcript or the provider request.
 */
export async function bootstrapHaciendaProjection(options = {}) {
  const gateway = loopbackOrigin(options.gatewayUrl ?? 'http://127.0.0.1:8787');
  const correlation = resolveCorrelation(options);
  const credential = resolveCredential(options);
  if (!correlation) throw new Error('No Hacienda projection correlation available');
  if (!credential) throw new Error('No Hacienda gateway credential available');
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(correlation)) {
    throw new Error('Invalid Hacienda projection correlation');
  }

  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (typeof fetchImpl !== 'function') throw new Error('No fetch implementation available');
  const authHeaders = {
    authorization: `Bearer ${credential}`,
    origin: gateway,
    [CORRELATION_HEADER]: correlation,
  };

  const response = await fetchImpl(`${gateway}/mod/session`, {
    method: 'POST',
    headers: { ...authHeaders, 'content-type': 'application/json' },
    body: '{}',
  });
  if (!response?.ok) throw new Error('Hacienda projection bootstrap was refused');
  const bootstrap = await response.json();
  const expiresAt = Date.parse(bootstrap.expiresAt);
  if (
    loopbackOrigin(bootstrap.origin) !== gateway
    || !/^cmods_[0-9a-f]{64}$/.test(bootstrap.sessionId)
    || !/^cmcap_[0-9a-f]{64}$/.test(bootstrap.capability)
    || !Number.isFinite(expiresAt)
    || expiresAt <= Date.now()
    || bootstrap.correlation !== correlation
  ) {
    throw new Error('Hacienda projection bootstrap failed validation');
  }

  const session = createProjectionSession({
    sessionId: bootstrap.sessionId,
    origin: gateway,
    expiresAt,
    verifyEvent: event => event?.type === 'Mapping'
      && event.sessionId === bootstrap.sessionId
      && event.origin === gateway
      && event.capability === bootstrap.capability
      && typeof event.token === 'string'
      && typeof event.original === 'string',
  });

  const controller = new AbortController();
  let closed = false;
  const streamPromise = (async () => {
    const stream = await fetchImpl(
      `${gateway}/mod/events?session=${encodeURIComponent(bootstrap.sessionId)}`,
      {
        headers: {
          ...authHeaders,
          [CAPABILITY_HEADER]: bootstrap.capability,
          accept: 'text/event-stream',
        },
        signal: controller.signal,
      },
    );
    if (!stream?.ok) return;
    await consumeEvents(stream, event => {
      if (closed || event.type !== 'Mapping') return;
      // Fail closed: the session re-checks origin/session/token; verifyEvent
      // additionally pins the per-session capability.
      session.acceptMapEvent(
        { ...event, token: normalizeToken(event.token) },
        Date.now(),
      );
    });
  })().catch(() => {
    // A lost projection stream leaves pseudonyms intact; never surface text.
  });

  const bridge = Object.freeze({
    sessionId: bootstrap.sessionId,
    expiresAt,
    /** Resolves once the initial local event stream has been consumed. */
    ready: streamPromise,
    project(text) {
      const { normalized, originals } = normalizeText(String(text ?? ''));
      return session.project(normalized)
        .map(part => ({
          text: part.revealed
            ? part.text.replace(/\u0000/g, '')
            : restoreTokens(part.text, originals),
          revealed: part.revealed,
        }))
        .filter(part => part.text.length > 0);
    },
    revoke() {
      if (closed) return;
      closed = true;
      controller.abort();
      session.revoke();
      fetchImpl(`${gateway}/mod/revoke?session=${encodeURIComponent(bootstrap.sessionId)}`, {
        method: 'POST',
        headers: { ...authHeaders, [CAPABILITY_HEADER]: bootstrap.capability },
      }).catch(() => {});
    },
    async close() {
      bridge.revoke();
      await streamPromise;
    },
  });
  return bridge;
}

/** Install the bridge before `register.js` loads. */
export async function installHaciendaProjection(options = {}) {
  const bridge = await bootstrapHaciendaProjection(options);
  globalThis.haciendaProjectionBridge = bridge;
  return bridge;
}
