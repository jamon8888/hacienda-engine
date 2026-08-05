/**
 * Starts one `hacienda serve` instance for the whole test run (vitest's
 * `globalSetup`, not a per-file `beforeAll`) — three test files each starting
 * their own server was correct but wasteful (three `cargo build` races, three
 * server processes). `HACIENDA_TEST_BASE_URL` is inherited by every worker
 * process vitest forks from this one.
 */

import { startHacienda, type HaciendaServer } from "./_helpers.js";

let server: HaciendaServer | undefined;

export async function setup(): Promise<void> {
  server = await startHacienda();
  process.env.HACIENDA_TEST_BASE_URL = server.baseUrl;
}

export async function teardown(): Promise<void> {
  server?.stop();
}
