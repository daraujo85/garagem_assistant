<script>
  import { untrack } from "svelte";
  // Orb em Canvas2D (spike M0-T09). Gradiente radial pulsante, sem Three.js (brainstorming 2.2).
  // Decisao: Canvas2D no spike. O que o M0 mede e custo de JANELA (CPU/RAM do WebView2 com
  // transparencia + always-on-top), nao a tecnica de render. Se o Canvas2D nao performar, ou se o
  // M1 precisar de efeitos (ruido, bandas FFT), trocar por fragment shader WebGL cru (~100 linhas).
  // Props:
  //   assistantState  Idle/Listening/Thinking/Speaking/Error (so cor/velocidade; FSM real e do M1).
  //                   Nome evita colisao com a rune `$state` (store_rune_conflict do Svelte 5).
  //   fps     alvo de frames/s (60, 30, 15...). Implementado como frame skip sobre o RAF.
  //   paused  pausa forcada do RAF (simula "janela oculta" quando a Page Visibility nao dispara).
  //   animate false = desenha 1 frame estatico (usado na janela main, para nao poluir a medicao).
  //   size    lado do canvas em px CSS (a janela do orb e 200x200).
  //   onstats callback ~1 Hz com { fps, running, hidden } (fps medido de verdade).
  let {
    assistantState = "Idle",
    fps = 60,
    paused = false,
    animate = true,
    size = 160,
    onstats = null,
  } = $props();

  const colors = {
    Idle: "#8a8a99",
    Listening: "#2ecc71",
    Thinking: "#f1c40f",
    Speaking: "#3498db",
    Error: "#e74c3c",
  };
  // Periodo do pulso (s) por estado: so para dar feedback visual no spike.
  const periods = { Idle: 3, Listening: 1.2, Thinking: 0.8, Speaking: 1.5, Error: 0.5 };

  let canvas;
  let ctx = null;
  // Page Visibility (brainstorming 2.2): oculta/minimizada -> RAF cancelado de vez.
  let hidden = $state(typeof document !== "undefined" ? document.hidden : false);
  let running = $derived(animate && !paused && !hidden);

  function rgba(hex, a) {
    const n = parseInt(hex.slice(1), 16);
    return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
  }

  function draw(tMs) {
    if (!ctx) return;
    const color = colors[assistantState] ?? "#555566";
    const period = periods[assistantState] ?? 3;
    // Pulso por tempo absoluto (nao por contagem de frames): a velocidade visual e a mesma a 60/30/15 fps.
    const pulse = 0.5 + 0.5 * Math.sin((tMs / 1000) * ((2 * Math.PI) / period));
    const c = size / 2;
    const r = size * 0.28 * (1 + 0.08 * pulse);

    ctx.clearRect(0, 0, size, size);

    // Halo (glow) externo, alpha varia com o pulso. Cabe no canvas: 1.5 * r < size / 2.
    const glow = ctx.createRadialGradient(c, c, r * 0.4, c, c, r * 1.5);
    glow.addColorStop(0, rgba(color, 0.35 + 0.25 * pulse));
    glow.addColorStop(1, rgba(color, 0));
    ctx.fillStyle = glow;
    ctx.beginPath();
    ctx.arc(c, c, r * 1.5, 0, Math.PI * 2);
    ctx.fill();

    // Nucleo com highlight deslocado (efeito de esfera).
    const core = ctx.createRadialGradient(c - r * 0.3, c - r * 0.3, r * 0.1, c, c, r);
    core.addColorStop(0, rgba("#ffffff", 0.55));
    core.addColorStop(0.35, rgba(color, 0.95));
    core.addColorStop(1, rgba(color, 0.75 + 0.2 * pulse));
    ctx.fillStyle = core;
    ctx.beginPath();
    ctx.arc(c, c, r, 0, Math.PI * 2);
    ctx.fill();
  }

  // Dimensiona o backing store pelo devicePixelRatio (limitado a 2: custo de fill cresce com dpr^2).
  $effect(() => {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(size * dpr);
    canvas.height = Math.round(size * dpr);
    ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    // untrack: so `size` deve redimensionar o canvas (assistantState/running tem efeito proprio abaixo).
    untrack(() => draw(0));
  });

  // Quando parado (static, pausado ou oculto), redesenha 1 frame a cada mudanca de estado/cor.
  $effect(() => {
    void assistantState;
    if (!running) untrack(() => draw(0));
  });

  // Loop RAF com frame skip. Reexecuta (e limpa o RAF anterior) quando fps/running mudam.
  $effect(() => {
    if (!running) {
      onstats?.({ fps: 0, running: false, hidden });
      return;
    }
    const interval = 1000 / Math.max(1, fps);
    let raf = 0;
    let last = 0;
    let frames = 0;
    let statStart = performance.now();

    function tick(t) {
      raf = requestAnimationFrame(tick);
      // Tolerancia de 2 ms: o RAF vem em multiplos de ~16.7 ms (60 Hz) e o jitter faria o 30 fps
      // virar 20 fps. Nota: o callback ainda acorda a 60 Hz; so o draw e pulado. Se o custo de acordar
      // pesar na medicao, trocar por setTimeout/RAF encadeado (a medir no Windows).
      if (t - last < interval - 2) return;
      last = t;
      draw(t);
      frames += 1;
      if (t - statStart >= 1000) {
        onstats?.({ fps: Math.round((frames * 1000) / (t - statStart)), running: true, hidden });
        frames = 0;
        statStart = t;
      }
    }
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  $effect(() => {
    const onVis = () => (hidden = document.hidden);
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  });
</script>

<!-- pointer-events: none -> cliques caem no container pai (data-tauri-drag-region no OrbWindow). -->
<canvas bind:this={canvas} style:width={`${size}px`} style:height={`${size}px`}></canvas>

<style>
  canvas {
    display: block;
    pointer-events: none;
  }
</style>
