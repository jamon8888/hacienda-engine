/**
 * Track C3's own check: a document processed through `redactPii` produces a real,
 * verifiable entry in the IndexedDB-backed audit chain — not a TypeScript
 * reimplementation of the chain, the actual `hacienda-core::audit::IndexedDbAuditStore`
 * (Track L5) compiled to wasm32, driven through `AuditHandle`
 * (`crates/hacienda-wasm/src/lib.rs`).
 *
 * `fake-indexeddb` is a spec-compliant IndexedDB implementation for Node, not a mock of
 * this project's own behavior — it stands in for the browser's `indexedDB` global the
 * same way `pii-engine.test.ts` loads the real `.wasm` bytes directly rather than going
 * through a dev server. What's under test (the chain's mint/append/verify logic,
 * `RedactionAuditEntry` → `AuditEntryInput` mapping, the wasm-bindgen boundary) is real
 * either way.
 */
import "fake-indexeddb/auto";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { describe, expect, it, beforeAll } from "vitest";
import initHaciendaWasm, {
  process as wasmProcess,
  scan as wasmScan,
  AuditHandle,
} from "hacienda-wasm";

const resolve = createRequire(import.meta.url).resolve;

beforeAll(async () => {
  const wasmPath = resolve("hacienda-wasm/hacienda_wasm_bg.wasm");
  await initHaciendaWasm({ module_or_path: readFileSync(wasmPath) });
});

const GENESIS_HASH =
  "0000000000000000000000000000000000000000000000000000000000000000";

describe("AuditHandle (Track C3)", () => {
  it("advances the chain tip past genesis when recording a redaction", async () => {
    const handle = await AuditHandle.open(
      "audit-handle-test-db-1",
      "test-node",
      "test-config",
    );
    expect(await handle.tip()).toBe(GENESIS_HASH);

    const result = await wasmProcess("IBAN FR7630006000011234567890189.");
    const tip = await handle.recordResult(result);

    expect(tip).not.toBe(GENESIS_HASH);
    expect(await handle.tip()).toBe(tip);
    await expect(handle.verify()).resolves.not.toThrow();
  });

  it("leaves the tip unchanged when the result has nothing to record", async () => {
    const handle = await AuditHandle.open(
      "audit-handle-test-db-2",
      "test-node",
      "test-config",
    );
    const tipBefore = await handle.tip();

    // `scan` never populates `audit_log` — nothing was redacted, so nothing to append.
    const result = await wasmScan("no pii here at all");
    const tipAfter = await handle.recordResult(result);

    expect(tipAfter).toBe(tipBefore);
  });

  it("survives a reload: a fresh handle on the same db name sees the prior entry", async () => {
    const dbName = "audit-handle-test-db-3";

    const handle = await AuditHandle.open(dbName, "test-node", "test-config");
    const result = await wasmProcess("Email: person@example.com.");
    const tipBefore = await handle.recordResult(result);

    const reopened = await AuditHandle.open(dbName, "test-node", "test-config");
    expect(await reopened.tip()).toBe(tipBefore);
    await expect(reopened.verify()).resolves.not.toThrow();
  });
});
