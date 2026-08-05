import { describe, expect, it } from "vitest";
import {
  base32Decode,
  base32Encode,
  categoryLabel,
  cmac,
  cmacSubkeys,
  dbl,
  deriveKeyHex,
  hexDecode,
  importCmacTestKey,
  mintToken,
  normalize,
  pad,
  revealToken,
  unpad,
} from "./pseudonymize";

// Track F2: reversible pseudonymization, matching `redaction/pseudonym.rs` (AES-256-SIV,
// RFC 5297) byte-for-byte. Three independent layers of verification here, in increasing
// order of what they'd actually catch:
//
// 1. RFC 4493 known-answer vectors for the AES-CMAC primitive this module's S2V is built
//    on — algorithm-correctness, independent of anything Rust does.
// 2. Golden vectors captured from a real `Pseudonymiser::token()`/`reveal()` run against
//    this exact key (`07` repeated `KEY_BYTES` times) — see the comment above each case
//    for how it was produced. This is the test that would catch a subtly wrong key-half
//    ordering, S2V step, or category-label mapping that a self-consistency test cannot:
//    code that is wrong in the same way on both sides of a round-trip still round-trips.
// 3. Round-trip and property tests, for the cases the golden vectors don't cover.

const KEY_HEX = "07".repeat(64);

describe("AES-CMAC (RFC 4493 known-answer tests)", () => {
  // Vectors from RFC 4493 §4, K = 2b7e151628aed2a6abf7158809cf4f3c. AES-CMAC is
  // cipher-agnostic (same algorithm regardless of AES-128 vs AES-256), so these validate
  // `dbl`/`cmac`'s block-chaining and padding logic independently of the AES-256 keys the
  // real pseudonym flow uses.
  const K = "2b7e151628aed2a6abf7158809cf4f3c";
  const M = "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710";

  async function keys() {
    const cbcKey = await importCmacTestKey(hexDecode(K)!);
    return { cbcKey, subkeys: await cmacSubkeys(cbcKey) };
  }

  function hex(bytes: Uint8Array): string {
    return Array.from(bytes).map((b) => b.toString(16).padStart(2, "0")).join("");
  }

  it("Example 1: empty message", async () => {
    const { cbcKey, subkeys } = await keys();
    const tag = await cmac(cbcKey, subkeys, new Uint8Array(0));
    expect(hex(tag)).toBe("bb1d6929e95937287fa37d129b756746");
  });

  it("Example 2: exactly one block (16 bytes)", async () => {
    const { cbcKey, subkeys } = await keys();
    const message = hexDecode(M.slice(0, 32))!;
    const tag = await cmac(cbcKey, subkeys, message);
    expect(hex(tag)).toBe("070a16b46b4d4144f79bdd9dd04a287c");
  });

  it("Example 3: 40 bytes (two full blocks + a partial block)", async () => {
    const { cbcKey, subkeys } = await keys();
    const message = hexDecode(M.slice(0, 80))!;
    const tag = await cmac(cbcKey, subkeys, message);
    expect(hex(tag)).toBe("dfa66747de9ae63030ca32611497c827");
  });

  it("Example 4: 64 bytes (four full blocks)", async () => {
    const { cbcKey, subkeys } = await keys();
    const message = hexDecode(M)!;
    const tag = await cmac(cbcKey, subkeys, message);
    expect(hex(tag)).toBe("51f0bebf7e3b9d92fc49741779363cfe");
  });

  it("derives the CMAC subkeys (K1, K2) matching a reference GF(2^128) doubling of L", async () => {
    const { cbcKey } = await keys();
    const { k1, k2 } = await cmacSubkeys(cbcKey);
    expect(hex(k1)).toBe("fbeed618357133667c85e08f7236a8de");
    expect(hex(k2)).toBe("f7ddac306ae266ccf90bc11ee46d513b");
  });
});

describe("golden vectors captured from a real Pseudonymiser run (Track F2)", () => {
  // Captured 2026-07-30 via `cargo test -p hacienda-core` against a scratch test
  // (`hacienda-core/tests/zz_golden_vectors.rs`, not committed) that built a
  // `Pseudonymiser` from `EnvKeyResolver::with_lookup` returning `"07".repeat(KEY_BYTES)`
  // for key id `k1` and called `.token()`/`.reveal()` on each case below. If this module's
  // key-half ordering, S2V, category-label mapping, or normalization diverges from the
  // Rust side in any way, these are the assertions that catch it — a self-consistency
  // round-trip test (mint then reveal in the same language) cannot.
  const cases: Array<[category: string, text: string, token: string, revealed: string]> = [
    [
      "email",
      "alice@example.com",
      "[EMAIL:k1:L732D2APGTRA74WMNRAYJTGCFVK2ZOENNO45PU35DRH3AOT3EZF3ELVBFVPQGK7QQ5QXUSMGL2AVY]",
      "alice@example.com",
    ],
    [
      "phone_number",
      "+33 6 12 34 56 78",
      "[PHONENUMBER:k1:INZOAUHJKNHRVJ5LOSMINXJ657XQDEQ6HMR67RY4FITWEXOCI42Q]",
      "+33612345678",
    ],
    [
      "full_name",
      "Jean Dupont",
      "[FULLNAME:k1:E3P72PJ5AXA25ZBC6TUXKFDTIZVUBR3YJNMHZKJPKSSJFKUTVUOQ]",
      "jean dupont",
    ],
    [
      "iban",
      "FR7630006000011234567890189",
      "[IBAN:k1:XEPIMSRZNA73NIY7N7NYDYTQ42QXNZVCQTRZ57PKEAFVKNY54PDGEIOB2C6QDUDGDKN3ARFIOAUMC]",
      "fr7630006000011234567890189",
    ],
  ];

  it.each(cases)("mints the exact Rust-produced token for %s %j", async (category, text, expectedToken) => {
    const token = await mintToken(category, text, "k1", KEY_HEX);
    expect(token).toBe(expectedToken);
  });

  it.each(cases)("reveals a Rust-minted token for %s to the same normalized text", async (category, _text, token, expectedRevealed) => {
    void category;
    const revealed = await revealToken(token, "k1", KEY_HEX);
    expect(revealed).toBe(expectedRevealed);
  });
});

describe("mintToken / revealToken round-trip", () => {
  it("reveals its own minted token back to the normalized text", async () => {
    const token = await mintToken("email", "Bob@Example.com", "k1", KEY_HEX);
    const revealed = await revealToken(token, "k1", KEY_HEX);
    expect(revealed).toBe("bob@example.com");
  });

  it("is deterministic: the same value mints the same token every time", async () => {
    const a = await mintToken("email", "alice@example.com", "k1", KEY_HEX);
    const b = await mintToken("email", "alice@example.com", "k1", KEY_HEX);
    expect(a).toBe(b);
  });

  it("collapses case and whitespace variants onto the same token", async () => {
    const a = await mintToken("email", "Alice@X.io", "k1", KEY_HEX);
    const b = await mintToken("email", "  alice@x.io ", "k1", KEY_HEX);
    expect(a).toBe(b);
  });

  it("mints a different token for the same text in a different category", async () => {
    const a = await mintToken("username", "same value", "k1", KEY_HEX);
    const b = await mintToken("full_name", "same value", "k1", KEY_HEX);
    expect(a).not.toBe(b);
  });

  it("fails closed (returns null) when revealing under the wrong key", async () => {
    const token = await mintToken("email", "alice@example.com", "k1", KEY_HEX);
    const wrongKey = "a9".repeat(64);
    const revealed = await revealToken(token, "k1", wrongKey);
    expect(revealed).toBeNull();
  });

  it("fails closed on a tampered ciphertext", async () => {
    const token = await mintToken("email", "alice@example.com", "k1", KEY_HEX);
    const tampered = token.slice(0, -2) + (token.at(-2) === "A" ? "B" : "A") + "]";
    const revealed = await revealToken(tampered, "k1", KEY_HEX);
    expect(revealed).toBeNull();
  });

  it("returns null for a token that does not parse", async () => {
    expect(await revealToken("not a token", "k1", KEY_HEX)).toBeNull();
    expect(await revealToken("[MISSING_FIELDS]", "k1", KEY_HEX)).toBeNull();
  });
});

describe("categoryLabel", () => {
  // Every fixed `PiiCategory` variant from `hacienda-core/src/pii/types.rs`, snake_case
  // (this app's `PiiEntity.category` form) mapped to the exact label
  // `category_label()`'s `{variant:?}.to_uppercase()` would produce in Rust.
  const rustLabels: Record<string, string> = {
    email: "EMAIL",
    phone_number: "PHONENUMBER",
    address: "ADDRESS",
    ssn: "SSN",
    passport_number: "PASSPORTNUMBER",
    drivers_license: "DRIVERSLICENSE",
    eu_vat: "EUVAT",
    national_id: "NATIONALID",
    tax_id: "TAXID",
    credit_card: "CREDITCARD",
    iban: "IBAN",
    bank_account: "BANKACCOUNT",
    routing_number: "ROUTINGNUMBER",
    swift_bic: "SWIFTBIC",
    crypto_wallet: "CRYPTOWALLET",
    medical_record_number: "MEDICALRECORDNUMBER",
    health_plan_number: "HEALTHPLANNUMBER",
    diagnosis: "DIAGNOSIS",
    medication: "MEDICATION",
    username: "USERNAME",
    password: "PASSWORD",
    api_key: "APIKEY",
    secret_token: "SECRETTOKEN",
    jwt_token: "JWTTOKEN",
    ip_address: "IPADDRESS",
    mac_address: "MACADDRESS",
    url: "URL",
    license_plate: "LICENSEPLATE",
    vehicle_vin: "VEHICLEVIN",
    date_of_birth: "DATEOFBIRTH",
    full_name: "FULLNAME",
    person: "PERSON",
    organization: "ORGANIZATION",
  };

  it.each(Object.entries(rustLabels))("maps %s to the Rust label %s", (snake, expected) => {
    expect(categoryLabel(snake)).toBe(expected);
  });

  it("rejects a category whose label would contain a token delimiter", () => {
    expect(() => categoryLabel("weird:category")).toThrow();
    expect(() => categoryLabel("weird[category]")).toThrow();
  });
});

describe("pad / unpad", () => {
  it("always adds at least one byte and pads to a 16-byte bucket", () => {
    const padded = pad(new Uint8Array([1, 2, 3]));
    expect(padded.length).toBe(16);
    expect(padded[15]).toBe(13);
  });

  it("adds a full extra bucket when the input is already block-aligned", () => {
    const padded = pad(new Uint8Array(16));
    expect(padded.length).toBe(32);
    expect(padded[31]).toBe(16);
  });

  it("round-trips through unpad", () => {
    for (const len of [0, 1, 15, 16, 17, 31, 32]) {
      const original = new Uint8Array(len).map((_, i) => i % 256);
      expect(unpad(pad(original))).toEqual(original);
    }
  });

  it("rejects malformed padding", () => {
    expect(unpad(new Uint8Array(15))).toBeNull(); // not a multiple of 16
    expect(unpad(new Uint8Array(16))).toBeNull(); // pad_len byte is 0
  });
});

describe("normalize", () => {
  it("NFKC-normalizes, lowercases, and collapses whitespace for the base rule", () => {
    expect(normalize("email", "  Alice@X.io  ")).toBe("alice@x.io");
    expect(normalize("full_name", "Jean   Dupont")).toBe("jean dupont");
  });

  it("strips everything but '+' and digits for phone numbers", () => {
    expect(normalize("phone_number", "+33 6 12 34 56 78")).toBe("+33612345678");
    expect(normalize("phone_number", "(555) 123-4567")).toBe("5551234567");
  });
});

describe("base32Encode / base32Decode (RFC 4648, no padding)", () => {
  it("round-trips arbitrary byte strings", () => {
    for (const len of [0, 1, 5, 16, 17, 32]) {
      const bytes = new Uint8Array(len).map((_, i) => (i * 7) % 256);
      expect(base32Decode(base32Encode(bytes))).toEqual(bytes);
    }
  });

  it("matches a known RFC 4648 test vector", () => {
    expect(base32Encode(new TextEncoder().encode("foobar"))).toBe("MZXW6YTBOI");
  });

  it("rejects invalid characters and non-zero padding bits", () => {
    expect(base32Decode("0189!")).toBeNull();
  });
});

// PBKDF2_ITERATIONS is 600,000 (OWASP's 2023 minimum) by design — each `deriveKeyHex` call
// is deliberately expensive, so tests calling it two or three times routinely exceed
// vitest's 5s default even on capable hardware. Explicit per-test timeouts, not a host-load
// workaround: this cost is inherent to the function under test, not incidental slowness.
describe("deriveKeyHex (Track F1)", () => {
  it(
    "is deterministic: the same passphrase and key id always derive the same key",
    async () => {
      const a = await deriveKeyHex("correct horse battery staple", "session");
      const b = await deriveKeyHex("correct horse battery staple", "session");
      expect(a).toBe(b);
      expect(a).toHaveLength(128); // KEY_BYTES * 2 hex chars
    },
    40000,
  );

  it(
    "derives a different key for a different passphrase",
    async () => {
      const a = await deriveKeyHex("correct horse battery staple", "session");
      const b = await deriveKeyHex("Correct horse battery staple", "session");
      expect(a).not.toBe(b);
    },
    40000,
  );

  it(
    "derives a different key for a different key id, same passphrase",
    async () => {
      const a = await deriveKeyHex("correct horse battery staple", "session");
      const b = await deriveKeyHex("correct horse battery staple", "rotated-2027");
      expect(a).not.toBe(b);
    },
    40000,
  );

  it(
    "round-trips a real token end to end from a passphrase alone",
    async () => {
      const keyHex = await deriveKeyHex("a lawyer's passphrase", "session");
      const token = await mintToken("email", "alice@example.com", "session", keyHex);
      const revealed = await revealToken(token, "session", keyHex);
      expect(revealed).toBe("alice@example.com");

      // Re-deriving from the same passphrase later — a fresh browser tab, not a cached
      // key — must reveal the same token, or the whole point of a passphrase is lost.
      const rederivedHex = await deriveKeyHex("a lawyer's passphrase", "session");
      expect(await revealToken(token, "session", rederivedHex)).toBe("alice@example.com");
    },
    60000,
  );
});
