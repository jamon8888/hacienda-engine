// Claude Code Mods adapter. This module intentionally uses only the Mods API
// for process settings/environment and HTTP; it has no Node.js imports.

import { createProjectionSession } from '../../claude-mod/src/renderer.js';

export const MODS_MIN_VERSION = '2.1.326';
export const CORRELATION_HEADER = 'x-hacienda-projection-correlation';
export const CAPABILITY_HEADER = 'x-hacienda-projection-capability';
const MAX_EVENTS = 32;
const MAX_RESPONSE_BYTES = 256 * 1024;
const MAX_ORIGINAL_BYTES = 4096;
const WIRE_TOKEN = /^\[([A-Z][A-Z0-9_]*):([A-Za-z0-9_-]+):([A-Za-z0-9_-]+)\]$/;
const WIRE_TOKENS = /\[([A-Z][A-Z0-9_]*):([A-Za-z0-9_-]+):([A-Za-z0-9_-]+)\]/g;
const MOD_TOKENS = /[A-Z][A-Z0-9_]*_[A-Z0-9_-]+/g;

function numericLoopbackOrigin(value) {
  let url;
  try { url = new URL(value); } catch { throw new Error('Invalid Hacienda gateway URL'); }
  const ip = url.hostname.split('.');
  if (
    url.protocol !== 'http:'
    || !/^127(?:\.\d{1,3}){3}$/.test(url.hostname)
    || ip.some(part => Number(part) > 255)
    || url.username || url.password
    || url.pathname !== '/' || url.search || url.hash
  ) {
    throw new Error('Hacienda gateway must be a numeric HTTP loopback origin');
  }
  return url.origin;
}

function correlationFromHeaders(value) {
  if (typeof value !== 'string') return undefined;
  for (const line of value.split(/\r?\n/)) {
    const index = line.indexOf(':');
    if (index < 0) continue;
    if (line.slice(0, index).trim().toLowerCase() === CORRELATION_HEADER) {
      return line.slice(index + 1).trim();
    }
  }
  return undefined;
}

function helperPathFromSetting(value, expectedPath) {
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  const unquoted = /^(['"])(.*)\1$/.exec(trimmed)?.[2] ?? trimmed;
  return unquoted === expectedPath ? unquoted : undefined;
}

async function resolveInputs($) {
  // hookEnv is available in v2.1.326+. It supplies the current plugin/session
  // context, not secrets; credentials and Hacienda's correlation hint are read
  // through the documented settings/env APIs below.
  const hookEnv = await $.process.hookEnv();
  if (typeof hookEnv?.CLAUDE_CODE_SESSION_ID !== 'string' || !hookEnv.CLAUDE_CODE_SESSION_ID) {
    throw new Error('Claude Code session context is unavailable');
  }

  const settings = await $.settings.read({ source: 'user' });
  const settingsEnv = settings?.env ?? {};
  const gatewayUrl = settingsEnv.ANTHROPIC_BASE_URL
    ?? await $.env.get('ANTHROPIC_BASE_URL')
    ?? 'http://127.0.0.1:8787';
  const origin = numericLoopbackOrigin(gatewayUrl);
  const correlation = correlationFromHeaders(
    settingsEnv.ANTHROPIC_CUSTOM_HEADERS ?? await $.env.get('ANTHROPIC_CUSTOM_HEADERS'),
  );
  if (!correlation || !/^[A-Za-z0-9_-]{1,128}$/.test(correlation)) {
    throw new Error('Hacienda projection correlation is unavailable');
  }

  let credential = settingsEnv.ANTHROPIC_AUTH_TOKEN
    ?? await $.env.get('ANTHROPIC_AUTH_TOKEN');
  if (typeof credential !== 'string' || !credential.trim()) {
    const home = await $.env.get('HOME');
    const configDir = await $.env.get('CLAUDE_CONFIG_DIR') || (home ? `${home}/.claude` : undefined);
    if (!configDir) throw new Error('Hacienda gateway credential is unavailable');
    const expectedHelper = `${configDir}/hacienda-gateway-key.sh`;
    const helper = helperPathFromSetting(settings.apiKeyHelper, expectedHelper);
    if (!helper) throw new Error('No validated Hacienda credential helper is configured');
    const result = await $.process.run([helper]);
    if (result?.exitCode !== 0 || typeof result.stdout !== 'string') {
      throw new Error('Hacienda gateway credential helper failed');
    }
    credential = result.stdout.trim();
  }
  if (typeof credential !== 'string' || !credential.trim() || /[\r\n]/.test(credential)) {
    throw new Error('Hacienda gateway credential is invalid');
  }
  return { origin, correlation, credential };
}

async function request($, url, init) {
  // The API routes guarantee no redirects; error mode is an additional guard
  // against any redirect support in the underlying fetch implementation.
  const response = await $.http.fetch(url, {
    ...init,
    redirect: 'error',
    cache: 'no-store',
  });
  if (!response?.ok || !Number.isInteger(response.status) || response.status < 200 || response.status >= 300) {
    throw new Error('Hacienda projection request was refused');
  }
  if (typeof response.text !== 'string' || new TextEncoder().encode(response.text).byteLength > MAX_RESPONSE_BYTES) {
    throw new Error('Hacienda projection response exceeded its limit');
  }
  return response.text;
}

function parseJson(text) {
  try { return JSON.parse(text); } catch { throw new Error('Malformed Hacienda projection response'); }
}

function normalizeToken(token) {
  const match = typeof token === 'string' && WIRE_TOKEN.exec(token);
  return match ? `${match[1]}_${match[2].toUpperCase()}_${match[3]}` : token;
}

/** Bootstrap once before a submitted prompt; poll one bounded batch at render. */
export async function createModProjection($) {
  const { origin: requestedOrigin, correlation, credential } = await resolveInputs($);
  const authHeaders = {
    authorization: `Bearer ${credential}`,
    origin: requestedOrigin,
    [CORRELATION_HEADER]: correlation,
  };
  const bootstrap = parseJson(await request($, `${requestedOrigin}/mod/session`, {
    method: 'POST',
    headers: { ...authHeaders, 'content-type': 'application/json' },
    body: '{}',
  }));

  const origin = numericLoopbackOrigin(bootstrap.origin);
  const expiresAt = Date.parse(bootstrap.expiresAt);
  if (
    origin !== requestedOrigin
    || !/^cmods_[0-9a-f]{64}$/.test(bootstrap.sessionId)
    || !/^cmcap_[0-9a-f]{64}$/.test(bootstrap.capability)
    || !Number.isFinite(expiresAt)
    || expiresAt <= Date.now()
    || bootstrap.correlation !== correlation
  ) {
    throw new Error('Hacienda projection bootstrap failed validation');
  }

  const sessionId = bootstrap.sessionId;
  const capability = bootstrap.capability;
  const session = createProjectionSession({
    sessionId,
    origin,
    expiresAt,
    verifyEvent: event => event?.type === 'Mapping'
      && event.origin === origin
      && event.sessionId === sessionId
      && event.capability === capability
      && typeof event.token === 'string'
      && typeof event.original === 'string',
  });
  let active = true;
  let pendingPoll;
  let remoteRevoke;

  const bridge = {
    expiresAt,
    isActive() { return active && Date.now() < expiresAt; },
    project(text) {
      if (!active || Date.now() >= expiresAt) return [{ text: String(text ?? ''), revealed: false }];
      const originals = new Map();
      WIRE_TOKENS.lastIndex = 0;
      const normalized = String(text ?? '').replace(WIRE_TOKENS, (wireToken, category, key, data) => {
        const token = `${category}_${key.toUpperCase()}_${data}`;
        originals.set(token, wireToken);
        // The NUL separator keeps adjacent Hacienda tokens independently
        // scannable by the renderer, which consumes the normalized forms.
        return `${token}\u0000`;
      });
      const projected = session.project(normalized);
      return projected.map(part => {
        const text = part.revealed ? part.text : restoreTokens(part.text, originals);
        return { text: text.replace(/\u0000/g, ''), revealed: part.revealed };
      }).filter(part => part.text.length > 0);
    },
    revoke() {
      if (!active) return remoteRevoke;
      active = false;
      session.revoke();
      remoteRevoke = $.http.fetch(`${origin}/mod/revoke?session=${encodeURIComponent(sessionId)}`, {
        method: 'POST',
        headers: {
          ...authHeaders,
          [CAPABILITY_HEADER]: capability,
        },
        redirect: 'error',
        cache: 'no-store',
      }).catch(() => undefined);
      return remoteRevoke;
    },
    async poll() {
      if (!active || Date.now() >= expiresAt) {
        bridge.revoke();
        return;
      }
      if (pendingPoll) return pendingPoll;
      pendingPoll = (async () => {
        try {
          const query = `session=${encodeURIComponent(sessionId)}&limit=${MAX_EVENTS}`;
          const text = await request($, `${origin}/mod/poll?${query}`, {
            method: 'GET',
            headers: {
              ...authHeaders,
              [CAPABILITY_HEADER]: capability,
              accept: 'application/json',
            },
          });
          const payload = parseJson(text);
          if (
            !payload || !Array.isArray(payload.events) || payload.events.length > MAX_EVENTS
            || typeof payload.expired !== 'boolean' || typeof payload.closed !== 'boolean'
          ) throw new Error('Invalid Hacienda mapping poll response');
          if (payload.expired || payload.closed) {
            bridge.revoke();
            return;
          }
          const encoder = new TextEncoder();
          for (const event of payload.events) {
            if (
              event?.type !== 'Mapping'
              || event.origin !== origin
              || event.sessionId !== sessionId
              || event.capability !== capability
              || typeof event.original !== 'string'
              || encoder.encode(event.original).byteLength > MAX_ORIGINAL_BYTES
              || typeof event.token !== 'string'
              || !WIRE_TOKEN.test(event.token)
            ) throw new Error('Invalid Hacienda mapping event');
            WIRE_TOKEN.lastIndex = 0;
            const accepted = session.acceptMapEvent({
              ...event,
              token: normalizeToken(event.token),
            });
            if (!accepted) throw new Error('Hacienda mapping event was rejected');
          }
        } catch {
          // Any transport, auth, redirect, parse, or validation failure clears
          // local originals; pseudonyms remain visible.
          bridge.revoke();
        } finally {
          pendingPoll = undefined;
        }
      })();
      return pendingPoll;
    },
  };
  return Object.freeze(bridge);
}

function restoreTokens(projected, originals) {
  if (originals.size === 0) return projected;
  MOD_TOKENS.lastIndex = 0;
  return projected.replace(MOD_TOKENS, token => originals.get(token) ?? token);
}
