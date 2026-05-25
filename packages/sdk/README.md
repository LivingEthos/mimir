# @mimir/sdk

TypeScript types generated from Mimir JSON Schemas.

## Usage

```typescript
import type {
  ContextPacket,
  ExecutablePatchPlan,
  ProviderCapabilitiesList,
} from '@mimir/sdk';

const packet: ContextPacket = {
  schema_version: 1,
  packet_id: "...",
  // ...
};

const recipe: ExecutablePatchPlan = {
  schema_version: 1,
  plan_id: "plan-123",
  packet_id: packet.packet_id,
  steps: [
    {
      action: "unified_diff",
      path: "src/lib.rs",
      diff: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ ...",
    },
  ],
};

const providers = {} as ProviderCapabilitiesList;
```

## Generation

The checked-in `.ts` files are generated from the root `schemas/*.schema.json` files with `json-schema-to-typescript`.

```bash
cd packages/sdk
npm run generate
npm run check:schema-drift
npm run build
```

`npm run generate` refreshes every schema mirror from `schemas/*.schema.json`. `npm run check:schema-drift` regenerates those mirrors in check mode and fails if the checked-in files are stale, then runs focused guard checks for important type shapes. `npm run build` rebuilds the bundled `index.d.ts` declaration file from the generated `.ts` files.
