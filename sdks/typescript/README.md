# @hacienda-engine/sdk

Official TypeScript client for the [hacienda-engine](https://github.com/jamon8888/hacienda-engine)
PII redaction and compliance API.

```ts
import { HaciendaClient } from "@hacienda-engine/sdk";

const client = new HaciendaClient({
  baseUrl: "https://your-hacienda-instance",
  apiKey: "hcd_live_...",
});

const result = await client.pii.scanText({
  text: "Contact Jane Doe at jane@example.com.",
});
for (const entity of result.entities) {
  console.log(entity.category, entity.start, entity.end);
}
```

Every method is a `Promise` — there is no separate sync client the way the Python
package has one, since JavaScript has no synchronous HTTP call to wrap.

## Development

See [`../README.md`](../README.md) for the shared `sdks/` layout. From the repo root:

```sh
npm run generate --workspace=sdks/typescript   # regenerates src/_generated/ (gitignored)
npm run test:unit --workspace=sdks/typescript
npm run lint --workspace=sdks/typescript
npm run build --workspace=sdks/typescript
```
