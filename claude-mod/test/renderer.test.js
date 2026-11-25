import test from 'node:test';
import assert from 'node:assert/strict';
import { createProjectionSession, installRenderer } from '../src/renderer.js';

// Expiry is a real epoch-millisecond timestamp in the future; the renderer
// validates this at construction rather than trusting callers to parse wire
// values. Tests pass explicit `now` values below FUTURE to model "live", and
// FUTURE itself to model "expired".
const FUTURE = Date.now() + 60_000;
const make = (expiresAt = FUTURE) => createProjectionSession({
  sessionId: 'session-1', origin: 'http://127.0.0.1:4312', expiresAt,
  verifyEvent: event => event.capability === 'valid'
});
const mapping = (extra = {}) => ({ type: 'Mapping', origin: 'http://127.0.0.1:4312',
  sessionId: 'session-1', token: 'PERSON_001', original: 'Ada Lovelace', capability: 'valid', ...extra });

test('projects authorized originals while preserving surrounding text', () => {
  const session = make();
  assert.equal(session.acceptMapEvent(mapping(), Date.now()), true);
  assert.deepEqual(session.project('Hello PERSON_001!', Date.now()), [
    { text: 'Hello ', revealed: false }, { text: 'Ada Lovelace', revealed: true },
    { text: '!', revealed: false }
  ]);
});

test('missing and expired sessions preserve pseudonyms and reject events', () => {
  assert.deepEqual(make().project('PERSON_001', Date.now()), [{ text: 'PERSON_001', revealed: false }]);
  const session = make();
  assert.equal(session.acceptMapEvent(mapping(), FUTURE), false);
  assert.deepEqual(session.project('PERSON_001', FUTURE), [{ text: 'PERSON_001', revealed: false }]);
});

test('rejects cross origin, cross session, and unauthenticated mapping events', () => {
  const session = make();
  for (const event of [mapping({ origin: 'http://evil.test' }), mapping({ sessionId: 'other' }),
    mapping({ capability: 'bad' })]) assert.equal(session.acceptMapEvent(event, Date.now()), false);
  assert.deepEqual(session.project('PERSON_001', Date.now()), [{ text: 'PERSON_001', revealed: false }]);
});

test('HTML-like originals remain plain text in safe Text nodes', async () => {
  const session = make();
  session.acceptMapEvent(mapping({ original: '<img src=x onerror=alert(1)>' }), Date.now());
  let hook;
  const renderSession = createProjectionSession({ sessionId: 'session-1', origin: 'http://127.0.0.1:4312',
    expiresAt: Number.MAX_SAFE_INTEGER, verifyEvent: event => event.capability === 'valid' });
  renderSession.acceptMapEvent(mapping({ original: '<img src=x onerror=alert(1)>' }));
  installRenderer((_name, _filter, handler) => { hook = handler; }, renderSession);
  const event = { props: { text: 'PERSON_001', isFirstOfReply: true, onScreen: true } };
  const output = await hook({ ui: { resolve: () => ({ Text: props => ({ type: 'Text', ...props }) }) }, event }, event,
    e => e);
  assert.deepEqual(output, { type: 'Text', children: [
    { type: 'Text', color: 'gray', children: ['<img src=x onerror=alert(1)>'] }
  ] });
});

test('no network by default and pseudonyms survive without an injected bridge', async () => {
  let hook;
  installRenderer((_name, _filter, handler) => { hook = handler; }, undefined);
  const event = { props: { text: 'PERSON_001' } };
  let called = false;
  const result = await hook({}, event, e => { called = true; return e; });
  assert.equal(called, true);
  assert.equal(result.props.text, 'PERSON_001');
});

test('invalid, non-finite, non-numeric and non-future expiresAt are refused', () => {
  const base = { sessionId: 'session-1', origin: 'http://127.0.0.1:4312',
    verifyEvent: () => true };
  for (const expiresAt of [
    undefined,
    null,
    '2030-01-01T00:00:00Z',        // raw RFC-3339 wire value → NaN when compared
    '5000',
    Number.NaN,
    Number.POSITIVE_INFINITY,
    Number.NEGATIVE_INFINITY,
    Date.now() - 1,                // already in the past
    0,
  ]) {
    assert.throws(
      () => createProjectionSession({ ...base, expiresAt }),
      TypeError,
      `expiresAt=${String(expiresAt)} must be refused`,
    );
  }
  // A valid finite future timestamp is accepted.
  assert.doesNotThrow(() => createProjectionSession({ ...base, expiresAt: FUTURE }));
});
