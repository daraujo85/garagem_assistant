<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import OrbVisualizer from "./components/OrbVisualizer.svelte";

  // Prova de IPC Rust <-> Svelte (M0-T01). Em M1 isso vira evento/Channel da FSM.
  let state = $state("carregando");
  let error = $state("");

  // Prova visual do spike PTT (M0-T03). Descartavel: em M1 o estado vem da FSM no Rust.
  // `ptt` e so o que o atalho global reporta (ocioso/escutando), independente de `state`.
  let ptt = $state("ocioso");
  let lastHoldMs = $state(null);
  let repeats = $state(0);
  let pressedCount = $state(0);
  let releasedCount = $state(0);
  // Orb reage ao atalho: Listening enquanto pressionado (cores definidas em OrbVisualizer).
  let orbState = $derived(ptt === "escutando" ? "Listening" : state);

  onMount(() => {
    // `listen` e async; guardamos as promises para desinscrever no cleanup (evita listener
    // duplicado com HMR do dev server).
    const unlistenPromises = [
      listen("shortcut-pressed", (event) => {
        ptt = "escutando";
        pressedCount += 1;
        console.log("[ptt] shortcut-pressed", event.payload, "recebido em", performance.now());
      }),
      listen("shortcut-released", (event) => {
        ptt = "ocioso";
        releasedCount += 1;
        lastHoldMs = event.payload.hold_ms;
        repeats = event.payload.repeats;
        console.log("[ptt] shortcut-released", event.payload, "recebido em", performance.now());
      }),
    ];

    invoke("get_state")
      .then((s) => (state = s))
      .catch((e) => {
        error = String(e);
        state = "Error";
      });

    return () => {
      unlistenPromises.forEach((p) => p.then((unlisten) => unlisten()));
    };
  });
</script>

<main>
  <OrbVisualizer state={orbState} />
  <p class="state">Estado: {state}</p>
  <section class="ptt">
    <p>PTT (Ctrl+Shift+Space): <strong>{ptt}</strong></p>
    <p>
      Ultimo hold:
      <strong>{lastHoldMs === null ? "-" : `${lastHoldMs.toFixed(1)} ms`}</strong>
      {#if repeats > 0}(repeticoes do SO: {repeats}){/if}
    </p>
    <p class="counts">Pressed: {pressedCount} | Released: {releasedCount}</p>
  </section>
  {#if error}<p class="error">{error}</p>{/if}
</main>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
    background: #14141c;
    color: #e6e6ee;
    font-family: system-ui, sans-serif;
  }
  main {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1.5rem;
  }
  .state {
    margin: 0;
    font-size: 1rem;
    opacity: 0.8;
  }
  .ptt {
    text-align: center;
    font-size: 0.95rem;
  }
  .ptt p {
    margin: 0.25rem 0;
  }
  .counts {
    opacity: 0.6;
    font-size: 0.8rem;
  }
  .error {
    color: #ff6b6b;
    font-size: 0.85rem;
  }
</style>
