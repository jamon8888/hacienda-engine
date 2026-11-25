import test from 'node:test';
import assert from 'node:assert/strict';

import { bootstrapHaciendaProjection, resolveCorrelation } from '../adapter.js';

const ORIGIN = 'http://127.0.0.1:8787';
const SESSION = `cmods_${'a'.repeat(64)}`;
const CAPABILITY = `cmcap_${'b'.repeat(64)}`;
const CORRELATION = 'hcm_test';
const CREDENTIAL = 'hcd_documents:process_test';
const TOKEN = '[EMAIL:k1:MFRGG2LTMVRXEZLU]';
const ORIGINAL = 'alice@example.com';

function mockFetch({ events = [], bootstrap = {}, streamOk = true } = {}) {
  const calls = [];
  const impl = async (url, init = {}) => {
    calls.push({ url: String(url), init });
    if (String(url).endsWith('/mod/session')) {
      return {
        ok: true,
        json: async () => ({
          sessionId: SESSION,
          origin: ORIGIN,
          expiresAt: new Date(Date.now() + 60_000).toISOString(),
          capability: CAPABILITY,
          correlation: CORRELATION,
          ...bootstrap,
        }),
      };
    }
    if (String(url).includes('/mod/events')) {
      const body = events
        .map(event => `data: ${JSON.stringify(event)}\n\n`)
        .join('');
      return {
        ok: streamOk,
        body: {
          getReader() {
            let done = false;
            return {
              async read() {
                if (done) return { done: true };
                done = true;
                return { done: false, value: new TextEncoder().encode(body) };
              },
            };
          },
        },
      };
    }
    if (String(url).includes('/mod/revoke')) return { ok: true };
    throw new Error(`unexpected URL ${url}`);
  };
  return { impl, calls };
}

const mapping = (extra = {}) => ({
  type: 'Mapping',
  origin: ORIGIN,
  sessionId: SESSION,
  token: TOKEN,
  original: ORIGINAL,
  capability: CAPABILITY,
  ...extra,
});

function options(mock, extra = {}) {
  return {
    gatewayUrl: ORIGIN,
    correlation: CORRELATION,
    credential: CREDENTIAL,
    fetch: mock.impl,
    ...extra,
  };
}

test('correlation is read from ANTHROPIC_CUSTOM_HEADERS without touching a credential', () => {
  assert.equal(
    resolveCorrelation({
      settings: { env: { ANTHROPIC_CUSTOM_HEADERS: `X-Org-Route: prod\nX-Hacienda-Projection-Correlation: ${CORRELATION}` } },
    }),
    CORRELATION,
  );
  assert.equal(resolveCorrelation({ settings: { env: {} } }), undefined);
});

test('local mapping reaches the Mod projection while upstream never sees the original', async () => {
  const mock = mockFetch({ events: [mapping()] });
  const bridge = await bootstrapHaciendaProjection(options(mock));
  await bridge.ready;

  const parts = bridge.project(`Reach ${TOKEN} or [EMAIL:k1:UNKNOWN].`);
  assert.ok(parts.some(part => part.revealed && part.text === ORIGINAL));
  const unrevealed = parts.filter(part => !part.revealed).map(part => part.text).join('');
  assert.ok(unrevealed.includes('[EMAIL:k1:UNKNOWN]'), 'unmapped token is restored verbatim');
  assert.ok(!unrevealed.includes(TOKEN), 'mapped token is not left as a pseudonym');
  assert.ok(!unrevealed.includes(ORIGINAL), 'original appears only in a revealed segment');
  for (const call of mock.calls) {
    assert.ok(!JSON.stringify(call).includes(ORIGINAL), 'original never leaves the projection');
    assert.equal(call.init.headers?.['x-hacienda-projection-correlation'], CORRELATION);
    assert.equal(call.init.headers?.authorization, `Bearer ${CREDENTIAL}`);
  }
  await bridge.close();
});

test('wrong capability, wrong session and unknown origin fail closed', async () => {
  const events = [
    mapping({ capability: `cmcap_${'c'.repeat(64)}` }),
    mapping({ sessionId: `cmods_${'d'.repeat(64)}` }),
    mapping({ origin: 'http://evil.test' }),
  ];
  const mock = mockFetch({ events });
  const bridge = await bootstrapHaciendaProjection(options(mock));
  await bridge.ready;
  assert.deepEqual(bridge.project(TOKEN), [{ text: TOKEN, revealed: false }]);
  await bridge.close();
});

test('expired or mismatched bootstrap is refused', async () => {
  const expired = mockFetch({
    bootstrap: { expiresAt: new Date(Date.now() - 1000).toISOString() },
  });
  await assert.rejects(() => bootstrapHaciendaProjection(options(expired)));
  const mismatch = mockFetch({ bootstrap: { correlation: 'hcm_other' } });
  await assert.rejects(() => bootstrapHaciendaProjection(options(mismatch)));
  const remote = mockFetch({ bootstrap: { origin: 'https://provider.example' } });
  await assert.rejects(() => bootstrapHaciendaProjection(options(remote)));
});

test('revoke drops mappings and posts a local revoke', async () => {
  const mock = mockFetch({ events: [mapping()] });
  const bridge = await bootstrapHaciendaProjection(options(mock));
  await bridge.ready;
  assert.ok(bridge.project(TOKEN).some(part => part.revealed));
  bridge.revoke();
  assert.deepEqual(bridge.project(TOKEN), [{ text: TOKEN, revealed: false }]);
  assert.ok(mock.calls.some(call => call.url.includes('/mod/revoke')));
});

test('missing correlation or credential never installs a bridge', async () => {
  const mock = mockFetch();
  await assert.rejects(() => bootstrapHaciendaProjection(options(mock, { correlation: '' })));
  await assert.rejects(() => bootstrapHaciendaProjection(options(mock, { credential: '' })));
  assert.equal(mock.calls.length, 0);
});
