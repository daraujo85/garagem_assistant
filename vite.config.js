import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Padrao do template Tauri 2: porta fixa (casa com devUrl do tauri.conf.json) e sem limpar a tela.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Evita recompilar o frontend quando o Rust muda.
      ignored: ["**/src-tauri/**"],
    },
  },
});
