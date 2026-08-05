import { beforeAll, describe, expect, it } from "vitest";
import { HaciendaApiError, HaciendaClient } from "../src/index.js";
import { getTestBaseUrl } from "./_helpers.js";

describe("whoami", () => {
  let client: HaciendaClient;

  beforeAll(() => {
    client = new HaciendaClient({ baseUrl: getTestBaseUrl(), apiKey: "test" });
  });

  it("reports every capability when auth is disabled", async () => {
    // The test fixture's `hacienda serve` runs with auth disabled (loopback
    // only), so every caller is `Caller::Trusted` — see
    // hacienda-api/src/handlers/auth.rs::whoami's doc comment: Trusted maps
    // to every capability, and principal_id is null.
    const response = await client.whoami();

    expect(response.principal_id).toBeNull();
    expect(response.capabilities).toContain("documents:process");
    expect(response.capabilities).toEqual([...response.capabilities].sort());
  });

  it("matches the direct namespace call", async () => {
    expect(await client.whoami()).toEqual(await client.auth.getWhoami());
  });

  it("raises HaciendaApiError on a 400", async () => {
    const error = await client.pii
      .revealToken({ token: "not-a-real-token" })
      .then(
        () => undefined,
        (reason: unknown) => reason,
      );

    expect(error).toBeInstanceOf(HaciendaApiError);
    if (!(error instanceof HaciendaApiError)) return;
    expect(error.statusCode).toBe(400);
    expect(error.code).toBe("invalid_request");
  });
});
