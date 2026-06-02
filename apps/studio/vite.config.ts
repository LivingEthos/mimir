import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiProxy = loopbackProxyTarget(env.VITE_MIMIR_API_PROXY);

  return {
    plugins: [react()],
    server: {
      host: "127.0.0.1",
      port: 5173,
      strictPort: false,
      proxy: apiProxy
        ? {
            "/v1": {
              target: apiProxy,
              changeOrigin: false,
              ws: true,
            },
          }
        : undefined,
    },
  };
});

function loopbackProxyTarget(value: string | undefined): string | undefined {
  const trimmed = value?.trim().replace(/\/$/, "");
  if (!trimmed) {
    return undefined;
  }

  const parsed = new URL(trimmed);
  if (!["http:", "https:"].includes(parsed.protocol)) {
    throw new Error("VITE_MIMIR_API_PROXY must use http or https");
  }
  if (!["127.0.0.1", "localhost", "::1"].includes(parsed.hostname)) {
    throw new Error("VITE_MIMIR_API_PROXY must target a loopback host");
  }
  return parsed.toString().replace(/\/$/, "");
}
