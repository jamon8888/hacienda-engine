import { beforeAll, describe, expect, it } from "vitest";
import { HaciendaClient } from "../src/index.js";
import { getTestBaseUrl } from "./_helpers.js";

describe("documents", () => {
  let client: HaciendaClient;

  beforeAll(() => {
    client = new HaciendaClient({ baseUrl: getTestBaseUrl(), apiKey: "test" });
  });

  it("processDocuments against a PII-disabled server returns no documents", async () => {
    // Known, already-documented pre-existing bug (CHANGELOG.md, Phase 13 Task
    // 3): `process_documents` zips extraction results with `result.pii`,
    // which is an empty Vec whenever no PII pipeline is configured (this
    // fixture's default `hacienda serve`) — so it silently returns
    // `documents: []` regardless of what was submitted. Pinned here (same as
    // the Python package's equivalent test) so a future fix updates this
    // assertion deliberately, rather than the SDK silently starting to
    // disagree with the API it wraps.
    const content = Buffer.from("hello world, no PII here").toString("base64");
    const result = await client.documents.processDocuments({
      documents: [
        {
          mime_type: "text/plain",
          content_base64: content,
          filename: "note.txt",
        },
      ],
    });

    expect(result.documents).toEqual([]);
  });

  it("processDocumentsBackground returns a pollable job", async () => {
    const content = Buffer.from("background job body").toString("base64");
    const accepted = await client.documents.processDocumentsBackground({
      documents: [{ mime_type: "text/plain", content_base64: content }],
    });

    expect(typeof accepted.job_id).toBe("string");
    expect(accepted.job_id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );

    const job = await client.jobs.getJob(accepted.job_id);
    expect(job.id).toBe(accepted.job_id);
  });
});
