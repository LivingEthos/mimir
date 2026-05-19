# @mimir/sdk

TypeScript types generated from Mimir JSON Schemas.

## Usage

```typescript
import { ContextPacket } from '@mimir/sdk/ContextPacket';

const packet: ContextPacket = {
  schema_version: 1,
  packet_id: "...",
  // ...
};
```

## Generation

Types are auto-generated from `schemas/*.schema.json` via `json-schema-to-typescript`.

```bash
cd packages/sdk
npm run build
```
