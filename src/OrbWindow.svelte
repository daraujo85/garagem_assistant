<script>
  // Raiz da janela "orb" (M0-T09): transparente, sem borda, always-on-top, 200x200 (tauri.conf.json).
  // Controlada pela janela main via evento "orb-config" ({ fps, paused, state }) e devolve telemetria
  // 1 Hz em "orb-stats". Estado FAKE: a FSM real e do M1.
  // O fps inicial pode vir por query param (`?fps=30`), mas a janela e aberta pelo tauri.conf.json sem
  // query; no spike use os botoes da janela main.
  import { listen, emit } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import OrbVisualizer from "./components/OrbVisualizer.svelte";

  const q = new URLSearchParams(location.search);
  let fps = $state(Number(q.get("fps")) || 60);
  let paused = $state(false);
  let state = $state(q.get("state") ?? "Idle");

  function onstats(s) {
    emit("orb-stats", s);
  }

  onMount(() => {
    const un = listen("orb-config", (e) => {
      const c = e.payload ?? {};
      if (c.fps !== undefined) fps = c.fps;
      if (c.paused !== undefined) paused = c.paused;
      if (c.state !== undefined) state = c.state;
    });
    return () => un.then((f) => f());
  });
</script>

<!-- data-tauri-drag-region: arrastar a janela sem borda (requer core:window:allow-start-dragging).
     Nao funciona com click-through ligado (o clique passa para baixo, e o esperado). -->
<div class="root" data-tauri-drag-region>
  <OrbVisualizer assistantState={state} {fps} {paused} size={200} {onstats} />
</div>

<style>
  /* Transparencia real: html/body sem fundo (o App.svelte global nao e carregado nesta janela). */
  :global(html, body) {
    margin: 0;
    height: 100%;
    background: transparent;
    overflow: hidden;
  }
  .root {
    width: 200px;
    height: 200px;
  }
</style>
