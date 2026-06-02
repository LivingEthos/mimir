# Mimir Studio

Local React/Vite UI for the `mimir serve --ui` API.

```bash
pnpm install
pnpm dev
pnpm typecheck
pnpm lint
pnpm test:smoke
```

Mock mode is the default when no UI token is present:

```text
http://127.0.0.1:5173/?mock=1
```

To point the dev server at a live loopback API, start `mimir serve --ui --port <port>`,
then run Vite with a same-origin proxy:

```bash
VITE_MIMIR_API_PROXY=http://127.0.0.1:<port> pnpm dev
```

Open the Vite URL with `?token=<printed-ui-token>`. The app strips the token from
the address bar and keeps it in memory only.
