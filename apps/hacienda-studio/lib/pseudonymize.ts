/**
 * Track F2: reversible pseudonymization, matching `hacienda-core`'s Rust implementation
 * (`redaction/pseudonym.rs`, AES-256-SIV / RFC 5297) byte-for-byte — a document
 * pseudonymized in Studio must be revealable by the CLI's `--mode pseudonymize` /
 * `hacienda reveal`, and vice versa. There is no wasm build of this path (unlike PII
 * detection/redaction, Track L6): reversible pseudonymization needs a 64-byte secret key
 * the browser must never receive from a server, so it has to run against WebCrypto
 * directly, in TypeScript, not the compiled Rust engine.
 *
 * WebCrypto has no native AES-SIV, AES-CMAC, or raw AES-ECB. Built from primitives it does
 * have: AES-CTR (native, matches RFC 5297's full-128-bit-counter `Ctr128BE` exactly with
 * `length: 128`) and AES-CBC with a zero IV as a single-block AES-ECB substitute — CBC's
 * first output block only depends on the IV and the first plaintext block, so encrypting
 * one 16-byte block with IV=0 and keeping only the first 16 output bytes (discarding
 * WebCrypto's automatic PKCS#7 padding block) is exactly one AES-ECB block encryption.
 * AES-CMAC (RFC 4493) and S2V (RFC 5297 §2.4) are built on top of that block primitive.
 *
 * Every piece here is traced directly against the vendored `aes-siv` crate source
 * (`aes-siv-0.7.0/src/siv.rs`) — key-half ordering, the S2V algorithm, the IV bit-masking
 * before CTR, and where the raw vs. masked tag is used — not reconstructed from the RFC
 * alone, because the RFC leaves some of that (e.g. exactly which 16-byte value gets stored
 * as the token's tag) as an implementation choice.
 */

const BUCKET = 16;
const BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/**
 * `Uint8Array.prototype.slice()`/`.subarray()` return a `Uint8Array<ArrayBufferLike>` (its
 * backing buffer could in principle be a `SharedArrayBuffer`), which TypeScript's DOM lib
 * no longer accepts where `BufferSource`/`crypto.subtle.*` expect a concrete
 * `Uint8Array<ArrayBuffer>`. Every array here really is plain-`ArrayBuffer`-backed; this
 * copies to re-assert that at each WebCrypto boundary rather than casting past the check.
 */
function fresh(bytes: Uint8Array): Uint8Array<ArrayBuffer> {
  return new Uint8Array(bytes);
}

// ---------------------------------------------------------------------------------------
// AES block primitive and GF(2^128) doubling
// ---------------------------------------------------------------------------------------

/** One AES-ECB block encryption, via AES-CBC(iv=0) discarding WebCrypto's padding block. */
async function aesEncryptBlock(
  cbcKey: CryptoKey,
  block: Uint8Array,
): Promise<Uint8Array<ArrayBuffer>> {
  const zeroIv = new Uint8Array(16);
  const out = await crypto.subtle.encrypt({ name: "AES-CBC", iv: zeroIv }, cbcKey, fresh(block));
  return fresh(new Uint8Array(out).slice(0, 16));
}

/**
 * GF(2^128) doubling with reduction polynomial x^128+x^7+x^2+x+1 (constant 0x87) — the
 * same operation `aes-siv`'s `dbl` crate provides, used both for CMAC subkey derivation
 * and inside S2V. `block` is treated as a big-endian 128-bit integer.
 */
// Exported for `pseudonymize.test.ts`'s RFC 4493 known-answer tests, which validate this
// module's AES block/CMAC primitives against published vectors independently of anything
// this codebase's own Rust side does — a bug shared between both `dbl` implementations
// would not show up in a Studio-vs-CLI round-trip test, but would in a KAT.
export function dbl(block: Uint8Array): Uint8Array {
  const out = new Uint8Array(16);
  const msb = (block[0] & 0x80) !== 0;
  for (let i = 0; i < 15; i++) {
    out[i] = ((block[i] << 1) & 0xff) | (block[i + 1] >>> 7);
  }
  out[15] = (block[15] << 1) & 0xff;
  if (msb) out[15] ^= 0x87;
  return out;
}

function xorBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length);
  for (let i = 0; i < a.length; i++) out[i] = a[i] ^ (b[i] ?? 0);
  return out;
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

// ---------------------------------------------------------------------------------------
// AES-CMAC (RFC 4493), generic over message length
// ---------------------------------------------------------------------------------------

interface CmacSubkeys {
  k1: Uint8Array;
  k2: Uint8Array;
}

export async function cmacSubkeys(cbcKey: CryptoKey): Promise<CmacSubkeys> {
  const l = await aesEncryptBlock(cbcKey, new Uint8Array(16));
  const k1 = dbl(l);
  const k2 = dbl(k1);
  return { k1, k2 };
}

/** Import a raw AES key for use with {@link cmac}/{@link cmacSubkeys} — the AES-CBC(iv=0)
 * single-block trick this module's CMAC is built on. Exported only for KAT testing; the
 * real token path uses {@link importSivKeys}, not this directly. */
export function importCmacTestKey(material: Uint8Array): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", fresh(material), { name: "AES-CBC" }, false, ["encrypt"]);
}

export async function cmac(
  cbcKey: CryptoKey,
  subkeys: CmacSubkeys,
  message: Uint8Array,
): Promise<Uint8Array> {
  const blockCount = message.length === 0 ? 1 : Math.ceil(message.length / BUCKET);
  const lastBlockIsComplete = message.length !== 0 && message.length % BUCKET === 0;

  const padded = new Uint8Array(blockCount * BUCKET);
  padded.set(message);
  if (!lastBlockIsComplete) {
    padded[message.length] = 0x80;
  }

  const lastStart = (blockCount - 1) * BUCKET;
  const lastBlock = padded.slice(lastStart, lastStart + BUCKET);
  const xorKey = lastBlockIsComplete ? subkeys.k1 : subkeys.k2;
  padded.set(xorBytes(lastBlock, xorKey), lastStart);

  let x = new Uint8Array(16);
  for (let i = 0; i < blockCount; i++) {
    const block = padded.slice(i * BUCKET, i * BUCKET + BUCKET);
    x = await aesEncryptBlock(cbcKey, xorBytes(x, block));
  }
  return x;
}

// ---------------------------------------------------------------------------------------
// S2V (RFC 5297 §2.4) — traced against `aes-siv-0.7.0/src/siv.rs`'s `s2v()`
// ---------------------------------------------------------------------------------------

async function s2v(
  cbcKey: CryptoKey,
  subkeys: CmacSubkeys,
  headers: Uint8Array[],
  message: Uint8Array,
): Promise<Uint8Array> {
  let state = await cmac(cbcKey, subkeys, new Uint8Array(16));

  for (const header of headers) {
    state = dbl(state);
    const code = await cmac(cbcKey, subkeys, header);
    state = xorBytes(state, code);
  }

  let tagInput: Uint8Array;
  if (message.length >= BUCKET) {
    const n = message.length - BUCKET;
    const prefix = message.slice(0, n);
    const lastPart = xorBytes(state, message.slice(n));
    tagInput = concatBytes(prefix, lastPart);
  } else {
    // Unreachable in this module's actual usage: `token()` always calls `pad()` first,
    // which guarantees a plaintext of at least BUCKET bytes — same guarantee the Rust
    // side documents at its own equivalent call site. Implemented anyway for parity with
    // the algorithm this is tracing, not because anything here can exercise it.
    state = dbl(state);
    const shortState = new Uint8Array(state);
    for (let i = 0; i < message.length; i++) shortState[i] ^= message[i];
    shortState[message.length] ^= 0x80;
    tagInput = shortState;
  }

  return cmac(cbcKey, subkeys, tagInput);
}

// ---------------------------------------------------------------------------------------
// AES-256-SIV encrypt/decrypt
// ---------------------------------------------------------------------------------------

interface SivKeys {
  /** First 32 bytes of the 64-byte key material — the CMAC/S2V key. */
  cbcKey: CryptoKey;
  subkeys: CmacSubkeys;
  /** Second 32 bytes — the CTR encryption key. Order per `Siv::new` in `aes-siv-0.7.0`. */
  ctrKey: CryptoKey;
}

export const KEY_BYTES = 64;

async function importSivKeys(material: Uint8Array): Promise<SivKeys> {
  if (material.length !== KEY_BYTES) {
    throw new Error(`pseudonym key must be ${KEY_BYTES} bytes, got ${material.length}`);
  }
  // `Siv::new`: "the first half of the key" (key[..32]) is the MAC key, the second half
  // (key[32..]) is the CTR encryption key — despite that function's own misleadingly
  // reversed comment; trust the slicing, not the prose, which was checked against the
  // vendored source directly.
  const macMaterial = material.slice(0, 32);
  const ctrMaterial = material.slice(32, 64);

  const cbcKey = await crypto.subtle.importKey(
    "raw",
    fresh(macMaterial),
    { name: "AES-CBC" },
    false,
    ["encrypt"],
  );
  const ctrKey = await crypto.subtle.importKey(
    "raw",
    fresh(ctrMaterial),
    { name: "AES-CTR" },
    false,
    ["encrypt", "decrypt"],
  );
  const subkeys = await cmacSubkeys(cbcKey);
  return { cbcKey, subkeys, ctrKey };
}

/** Zero the top bit of the 3rd and 4th 32-bit words before using a tag as a CTR IV — SIV's
 * mitigation against IV collision with the counter's carry behavior (RFC 5297 / the SIV
 * paper, §"we zero-out the top bit in each of the last two 32-bit words"). Does not
 * mutate `tag`: the *stored* token tag is the raw, unmasked S2V output. */
function maskForCtr(tag: Uint8Array): Uint8Array {
  const iv = new Uint8Array(tag);
  iv[8] &= 0x7f;
  iv[12] &= 0x7f;
  return iv;
}

async function ctrXor(ctrKey: CryptoKey, counter: Uint8Array, data: Uint8Array): Promise<Uint8Array> {
  if (data.length === 0) return new Uint8Array(0);
  const out = await crypto.subtle.encrypt(
    { name: "AES-CTR", counter: fresh(counter), length: 128 },
    ctrKey,
    fresh(data),
  );
  return new Uint8Array(out);
}

async function sivEncrypt(
  keys: SivKeys,
  headers: Uint8Array[],
  plaintext: Uint8Array,
): Promise<Uint8Array> {
  const tag = await s2v(keys.cbcKey, keys.subkeys, headers, plaintext);
  const body = await ctrXor(keys.ctrKey, maskForCtr(tag), plaintext);
  return concatBytes(tag, body);
}

/** Returns `null` on authentication failure — a wrong key, an altered header, or a forged
 * ciphertext, deliberately indistinguishable (see `PseudonymError::UnreadableToken`'s Rust
 * doc comment: distinguishing them would tell an attacker which guess was closer). */
async function sivDecrypt(
  keys: SivKeys,
  headers: Uint8Array[],
  ciphertext: Uint8Array,
): Promise<Uint8Array | null> {
  if (ciphertext.length < 16) return null;
  const tag = ciphertext.slice(0, 16);
  const body = ciphertext.slice(16);
  const plaintext = await ctrXor(keys.ctrKey, maskForCtr(tag), body);
  const recomputed = await s2v(keys.cbcKey, keys.subkeys, headers, plaintext);
  let diff = 0;
  for (let i = 0; i < 16; i++) diff |= tag[i] ^ recomputed[i];
  return diff === 0 ? plaintext : null;
}

// ---------------------------------------------------------------------------------------
// PKCS#7-to-BUCKET padding — matches `redaction/pseudonym.rs`'s `pad`/`unpad` exactly,
// including that a whole-bucket input still gains one full extra bucket of padding.
// ---------------------------------------------------------------------------------------

export function pad(bytes: Uint8Array): Uint8Array {
  const padLen = BUCKET - (bytes.length % BUCKET);
  const out = new Uint8Array(bytes.length + padLen);
  out.set(bytes);
  out.fill(padLen, bytes.length);
  return out;
}

export function unpad(bytes: Uint8Array): Uint8Array | null {
  if (bytes.length === 0 || bytes.length % BUCKET !== 0) return null;
  const padLen = bytes[bytes.length - 1];
  if (padLen === 0 || padLen > BUCKET || padLen > bytes.length) return null;
  const split = bytes.length - padLen;
  for (let i = split; i < bytes.length; i++) {
    if (bytes[i] !== padLen) return null;
  }
  return bytes.slice(0, split);
}

// ---------------------------------------------------------------------------------------
// Normalization — matches `redaction/pseudonym.rs`'s `normalize`/`base_normalize`.
// Known limitation shared with the Rust side: JS `String.prototype.toLowerCase()` is
// Unicode lowercasing, not full case-folding, same imperfect-for-a-few-scripts caveat the
// Rust doc comment already names.
// ---------------------------------------------------------------------------------------

export function normalize(category: string, text: string): string {
  if (category === "phone_number") {
    return Array.from(text)
      .filter((c) => c === "+" || (c >= "0" && c <= "9"))
      .join("");
  }
  return baseNormalize(text);
}

function baseNormalize(text: string): string {
  const collapsed = text.normalize("NFKC").toLowerCase().trim();
  if (collapsed === "") return "";
  return collapsed.split(/\s+/).join(" ");
}

// ---------------------------------------------------------------------------------------
// Base32, no padding (RFC 4648) — matches `data_encoding::BASE32_NOPAD`.
// ---------------------------------------------------------------------------------------

export function base32Encode(bytes: Uint8Array): string {
  let bits = 0;
  let value = 0;
  let output = "";
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      output += BASE32_ALPHABET[(value >>> (bits - 5)) & 0x1f];
      bits -= 5;
    }
  }
  if (bits > 0) {
    output += BASE32_ALPHABET[(value << (5 - bits)) & 0x1f];
  }
  return output;
}

export function base32Decode(input: string): Uint8Array | null {
  let bits = 0;
  let value = 0;
  const out: number[] = [];
  for (const ch of input.toUpperCase()) {
    const idx = BASE32_ALPHABET.indexOf(ch);
    if (idx === -1) return null;
    value = (value << 5) | idx;
    bits += 5;
    if (bits >= 8) {
      out.push((value >>> (bits - 8)) & 0xff);
      bits -= 8;
    }
  }
  // Any leftover bits must be zero-padding, not encoded data — an encoder never emits a
  // partial trailing symbol with non-zero low bits.
  if (bits >= 8 || (value & ((1 << bits) - 1)) !== 0) return null;
  return new Uint8Array(out);
}

// ---------------------------------------------------------------------------------------
// Token minting/reveal — matches `Pseudonymiser::token`/`reveal` and `category_label`.
// ---------------------------------------------------------------------------------------

export class PseudonymKeyError extends Error {}

/**
 * `hacienda-core`'s `category_label()` renders a `PiiCategory` via `{variant:?}.to_uppercase()`
 * — the derived-Debug variant name (e.g. `PhoneNumber`), not the serde `snake_case` form
 * this app's `PiiEntity.category` actually carries. Stripping underscores and uppercasing
 * the snake_case form recovers exactly that: `phone_number` -> `PhoneNumber` (Debug) ->
 * `PHONENUMBER` (uppercased) is the same string as `phone_number` with `_` removed and
 * uppercased, for every category in `PiiCategory` — the two casings are the same word
 * sequence, just delimited differently. Verified against every fixed variant in
 * `hacienda-core/src/redaction/pseudonym.rs`'s own test module (see `pseudonymize.test.ts`).
 */
export function categoryLabel(category: string): string {
  const label = category.replace(/_/g, "").toUpperCase();
  if (label === "" || label.includes(":") || label.includes("[") || label.includes("]")) {
    throw new PseudonymKeyError(
      `category '${category}' cannot appear in a pseudonym token`,
    );
  }
  return label;
}

/** A validated `[a-z0-9_]{1,16}` key id — same charset `KeyId::new` enforces in Rust. */
export function isValidKeyId(id: string): boolean {
  return id.length > 0 && id.length <= 16 && /^[a-z0-9_]+$/.test(id);
}

const keyCache = new Map<string, Promise<SivKeys>>();

/** `keyHex` is `KEY_BYTES` (64) bytes of lowercase hex — same encoding `EnvKeyResolver`
 * expects from `HACIENDA_PSEUDONYM_KEY_<ID>`. Cached per hex string so repeated tokens
 * under the same key do not re-run key import + CMAC subkey derivation per call. */
function keysFor(keyHex: string): Promise<SivKeys> {
  let cached = keyCache.get(keyHex);
  if (!cached) {
    const material = hexDecode(keyHex);
    if (!material || material.length !== KEY_BYTES) {
      throw new PseudonymKeyError(`pseudonym key must be ${KEY_BYTES * 2} lowercase hex characters`);
    }
    cached = importSivKeys(material);
    keyCache.set(keyHex, cached);
  }
  return cached;
}

export function hexDecode(hex: string): Uint8Array | null {
  const trimmed = hex.trim();
  if (trimmed.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(trimmed)) return null;
  const out = new Uint8Array(trimmed.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(trimmed.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

/**
 * Mint the token for `text` in `category`, under the key named by `keyId` with material
 * `keyHex`. Token shape: `[LABEL:key_id:base32]` — identical to what
 * `hacienda extract --mode pseudonymize` or `hacienda-api`'s reveal-gated routes would
 * produce for the same key and input, so a document pseudonymized here is revealable by
 * either.
 */
export async function mintToken(
  category: string,
  text: string,
  keyId: string,
  keyHex: string,
): Promise<string> {
  if (!isValidKeyId(keyId)) {
    throw new PseudonymKeyError(`invalid pseudonym key id '${keyId}'`);
  }
  const label = categoryLabel(category);
  const keys = await keysFor(keyHex);
  const plaintext = pad(encoder.encode(normalize(category, text)));
  const ciphertext = await sivEncrypt(keys, [encoder.encode(label)], plaintext);
  return `[${label}:${keyId}:${base32Encode(ciphertext)}]`;
}

/**
 * Recover the normalized value behind a token — matches `Pseudonymiser::reveal`. Returns
 * `null` for anything that doesn't parse as `[LABEL:key_id:base32]`, and throws
 * `PseudonymKeyError` only when the token is well-formed but no key with that id was
 * supplied (the caller is expected to look up the right key by id first).
 */
export async function revealToken(
  token: string,
  keyId: string,
  keyHex: string,
): Promise<string | null> {
  const body = token.startsWith("[") && token.endsWith("]") ? token.slice(1, -1) : null;
  if (body === null) return null;
  const firstColon = body.indexOf(":");
  const secondColon = body.indexOf(":", firstColon + 1);
  if (firstColon <= 0 || secondColon === -1) return null;
  const label = body.slice(0, firstColon);
  const tokenKeyId = body.slice(firstColon + 1, secondColon);
  const encoded = body.slice(secondColon + 1);
  if (!isValidKeyId(tokenKeyId)) return null;
  if (tokenKeyId !== keyId) {
    throw new PseudonymKeyError(
      `token was minted under key '${tokenKeyId}', not '${keyId}'`,
    );
  }

  const ciphertext = base32Decode(encoded);
  if (!ciphertext) return null;

  const keys = await keysFor(keyHex);
  const padded = await sivDecrypt(keys, [encoder.encode(label)], ciphertext);
  if (!padded) return null;
  const plaintext = unpad(padded);
  if (!plaintext) return null;
  try {
    return decoder.decode(plaintext);
  } catch {
    return null;
  }
}
