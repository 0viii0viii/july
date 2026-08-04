import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import pkg from "./package.json";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // 화면에 버전을 띄우기 위해 빌드 시점에 박아 넣는다. 런타임 API로 읽을 수도
  // 있지만 그건 권한(ACL)이 걸려 있어, 정작 문제를 진단해야 할 때 같이 막힐 수
  // 있다. 버전 표시만큼은 무조건 보여야 한다.
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  //    (1420은 다른 프로젝트가 쓰고 있어 1520으로 옮겼다. tauri.conf.json의
  //     devUrl과 반드시 같이 맞춰야 한다.)
  server: {
    port: 1520,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1521,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
