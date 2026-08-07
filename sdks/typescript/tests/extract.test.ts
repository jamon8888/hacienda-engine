import { beforeAll, describe, expect, it } from "vitest";
import { HaciendaClient } from "../src/index.js";
import { getTestBaseUrl } from "./_helpers.js";

describe("documents", () => {
  let client: HaciendaClient;

  beforeAll(() => {
    client = new HaciendaClient({ baseUrl: getTestBaseUrl(), apiKey: "test" });
  });

  it("processDocuments against a PII-disabled server still returns extraction results", async () => {
    // `process_documents` used to zip extraction results with `result.pii`, which is
    // an empty Vec whenever no PII pipeline is configured (this fixture's default
    // `hacienda serve`) — the zip silently dropped every document regardless of what
    // was submitted. Fixed server-side; this asserts the corrected behaviour: the
    // document is still returned, just with no entities.
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

    expect(result.documents).toHaveLength(1);
    expect(result.documents[0].entities).toEqual([]);
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
