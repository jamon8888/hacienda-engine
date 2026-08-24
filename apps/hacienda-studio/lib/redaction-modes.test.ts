/**
 * `hashSpanForProcessing` is the Hash-mode redaction that gets baked into exported
 * markdown, so its security property — that the digest is keyed, and therefore not
 * recoverable by enumerating low-entropy PII — is the thing worth pinning. A bare
 * SHA-256 would pass every determinism test here and still be broken; the
 * `different keys must produce different digests` case is what actually distinguishes
 * the two.
 */
import { createHmac } from "node:crypto";
import { describe, expect, it } from "vitest";
import { hashSpanForProcessing } from "./redaction-modes";

const KEY_A = "07".repeat(32);
const KEY_B = "5a".repeat(32);

describe("hashSpanForProcessing (Hash mode, keyed)", () => {
  it("is deterministic for the same category, text and key", async () => {
    const a = await hashSpanForProcessing("email", "alice@example.com", KEY_A);
    const b = await hashSpanForProcessing("email", "alice@example.com", KEY_A);
    expect(a).toBe(b);
  });

  it("produces different digests under different keys", async () => {
    // The security property: without this, an unsalted digest of a 9-digit SSN is
    // recovered by hashing all 10^9 candidates.
    const a = await hashSpanForProcessing("ssn", "123-45-6789", KEY_A);
    const b = await hashSpanForProcessing("ssn", "123-45-6789", KEY_B);
    expect(a).not.toBe(b);
  });

  it("separates categories so the same text hashes differently per category", async () => {
    const asEmail = await hashSpanForProcessing("email", "shared-value", KEY_A);
    const asName = await hashSpanForProcessing("full_name", "shared-value", KEY_A);
    expect(asEmail).not.toBe(asName);
  });

  it("links repeated occurrences of one value — the point of Hash mode", async () => {
    const first = await hashSpanForProcessing("full_name", "Jean Dupont", KEY_A);
    const second = await hashSpanForProcessing("full_name", "Jean Dupont", KEY_A);
    expect(first).toBe(second);
  });

  it("emits the documented `#category:<16 hex>` shape", async () => {
    const token = await hashSpanForProcessing("iban", "GB82WEST12345698765432", KEY_A);
    expect(token).toMatch(/^#iban:[0-9a-f]{16}$/);
  });

  it("matches an independent HMAC-SHA256 implementation", async () => {
    // Cross-checks the WebCrypto path against Node's own HMAC, so a mistake in key
    // import or message framing shows up as a mismatch rather than as a digest that is
    // merely self-consistent.
    const expected = createHmac("sha256", Buffer.from(KEY_A, "hex"))
      .update("email:alice@example.com")
      .digest("hex")
      .slice(0, 16);
    const token = await hashSpanForProcessing("email", "alice@example.com", KEY_A);
    expect(token).toBe(`#email:${expected}`);
  });
});
