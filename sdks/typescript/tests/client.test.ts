import { beforeAll, describe, expect, it } from "vitest";
import { HaciendaClient } from "../src/index.js";
import { getTestBaseUrl } from "./_helpers.js";

describe("HaciendaClient", () => {
  let client: HaciendaClient;
  let baseUrl: string;

  beforeAll(() => {
    baseUrl = getTestBaseUrl();
    client = new HaciendaClient({ baseUrl, apiKey: "test" });
  });

  it("fetches health and info", async () => {
    const health = await client.info.getHealth();
    expect(health.status).toBe("ok");

    const info = await client.info.getInfo();
    expect(info.name).toBe("hacienda-api");
  });

  it("rejects an unsupported target", () => {
    expect(
      () =>
        new HaciendaClient({
          baseUrl,
          apiKey: "test",
          // @ts-expect-error deliberately passing an unsupported target
          target: "device",
        }),
    ).toThrow(/cloud/);
  });
});
