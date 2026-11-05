<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen, emit } from "@tauri-apps/api/event";
  import { Window } from "@tauri-apps/api/window";
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

  // Spike STT (M0-T06): botao manual que transcreve o WAV mais recente de ./captures/.
  // Descartavel: em M1 a transcricao e disparada pela FSM ao soltar o atalho.
  let sttBusy = $state(false);
  let sttResult = $state(null);
  let sttError = $state("");

  async function transcribeLast() {
    sttBusy = true;
    sttError = "";
    try {
      sttResult = await invoke("transcribe_last_capture");
    } catch (e) {
      sttResult = null;
      sttError = String(e);
    } finally {
      sttBusy = false;
    }
  }

  // Spike TTS (M0-T07): texto + botao "Falar" -> `synthesize_speech` (Piper sidecar) -> tag <audio>.
  // Descartavel: em M1 o playback e em Rust (rodio/cpal, M0-T08) com streaming por frase.
  let ttsText = $state("Olá, como posso ajudar?");
  let ttsBusy = $state(false);
  let ttsResult = $state(null);
  let ttsError = $state("");
  let audioUrl = $state(null);

  async function speak() {
    ttsBusy = true;
    ttsError = "";
    try {
      const res = await invoke("synthesize_speech", { text: ttsText });
      ttsResult = res;
      // Busca os bytes do WAV e monta um blob URL para a tag <audio> (sem asset protocol).
      const bytes = await invoke("read_synthesized_wav", { path: res.wav_path });
      if (audioUrl) URL.revokeObjectURL(audioUrl);
      audioUrl = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
    } catch (e) {
      ttsResult = null;
      ttsError = String(e);
    } finally {
      ttsBusy = false;
    }
  }

  // Spike playback (M0-T08): toca o WAV do TTS em Rust (rodio ou cpal) e mede o stop. Descartavel.
  // `play_wav_rust` / `stop_playback` / `spike_conflict_test` em audio/playback.rs. A tag <audio>
  // acima fica para comparacao: ela nao permite medir a latencia de stop do lado Rust.
  let pbBackend = $state("rodio");
  let pbBusy = $state(false);
  let pbInfo = $state(null);
  let pbStop = $state(null);
  let pbError = $state("");
  let conflict = $state(null);

  async function playRust() {
    if (!ttsResult) return;
    pbBusy = true;
    pbError = "";
    pbStop = null;
    conflict = null;
    try {
      pbInfo = await invoke("play_wav_rust", { path: ttsResult.wav_path, backend: pbBackend });
    } catch (e) {
      pbInfo = null;
      pbError = String(e);
    } finally {
      pbBusy = false;
    }
  }

  async function stopRust() {
    pbError = "";
    try {
      // `performance.now()` so da o round-trip IPC; a medida valida e `stop_latency_ms` (log Rust).
      pbStop = await invoke("stop_playback");
    } catch (e) {
      pbStop = null;
      pbError = String(e);
    }
  }

  // Teste de conflito in+out (pergunta A): captura + playback juntos, por backend. Bloqueia a UI
  // pela duracao do WAV + ~0,5 s.
  async function conflictTest() {
    if (!ttsResult) return;
    pbBusy = true;
    pbError = "";
    pbInfo = null;
    pbStop = null;
    try {
      conflict = await invoke("spike_conflict_test", {
        path: ttsResult.wav_path,
        backend: pbBackend,
      });
    } catch (e) {
      conflict = null;
      pbError = String(e);
    } finally {
      pbBusy = false;
    }
  }

  // Spike janela/orb (M0-T09): controle da janela "orb" (transparente, always-on-top; ver
  // tauri.conf.json e OrbWindow.svelte). A janela main so manda config/comandos e mostra a telemetria
  // de fps; CPU/RAM se medem no Gerenciador de Tarefas. Estado do orb FAKE (FSM real e do M1).
  let orbFps = $state(60);
  let orbPaused = $state(false);
  let orbFakeState = $state("Idle");
  let orbClickThrough = $state(false);
  let orbVisible = $state(true);
  let orbStats = $state(null);
  let orbError = $state("");

  function sendOrbConfig(patch) {
    emit("orb-config", patch);
  }
  function setOrbFps(v) {
    orbFps = v;
    sendOrbConfig({ fps: v });
  }
  function toggleOrbPause() {
    orbPaused = !orbPaused;
    sendOrbConfig({ paused: orbPaused });
  }
  function setOrbFakeState(v) {
    orbFakeState = v;
    sendOrbConfig({ state: v });
  }
  async function orbWindow() {
    const w = await Window.getByLabel("orb");
    if (!w) throw new Error('janela "orb" nao encontrada (ver tauri.conf.json)');
    return w;
  }
  // Click-through: com `true` o orb deixa de receber mouse (cliques passam para o app de baixo) e so
  // este botao desliga. Ver docs/spikes.md (M0-T09).
  async function toggleClickThrough() {
    orbError = "";
    try {
      const next = !orbClickThrough;
      await (await orbWindow()).setIgnoreCursorEvents(next);
      orbClickThrough = next;
    } catch (e) {
      orbError = String(e);
    }
  }
  // hide() = janela oculta de verdade (Page Visibility deve pausar o RAF). Alternativa a minimize().
  async function toggleOrbVisible() {
    orbError = "";
    try {
      const w = await orbWindow();
      if (orbVisible) await w.hide();
      else await w.show();
      orbVisible = !orbVisible;
    } catch (e) {
      orbError = String(e);
    }
  }
  async function minimizeOrb() {
    orbError = "";
    try {
      await (await orbWindow()).minimize();
    } catch (e) {
      orbError = String(e);
    }
  }
  async function restoreOrb() {
    orbError = "";
    try {
      const w = await orbWindow();
      await w.unminimize();
      await w.show();
      orbVisible = true;
    } catch (e) {
      orbError = String(e);
    }
  }

  onMount(() => {
    // `listen` e async; guardamos as promises para desinscrever no cleanup (evita listener
    // duplicado com HMR do dev server).
    const unlistenPromises = [
      listen("orb-stats", (event) => {
        orbStats = event.payload;
      }),
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
  <!-- animate=false: o orb animado de verdade vive na janela "orb" (M0-T09); aqui so 1 frame estatico, para nao poluir a medicao. -->
  <OrbVisualizer state={orbState} animate={false} />
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
  <section class="stt">
    <button onclick={transcribeLast} disabled={sttBusy}>
      {sttBusy ? "Transcrevendo..." : "Transcrever ultima captura"}
    </button>
    {#if sttResult}
      <p class="stt-text">"{sttResult.text}"</p>
      <p class="counts">
        {sttResult.model} | audio {sttResult.audio_secs.toFixed(2)} s |
        processamento {sttResult.processing_ms} ms | RTF {sttResult.rtf.toFixed(2)} |
        idioma {sttResult.language}
      </p>
    {/if}
    {#if sttError}<p class="error">{sttError}</p>{/if}
  </section>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="tts">
    <h3>TTS (Piper sidecar)</h3>
    <textarea
      rows="3"
      bind:value={ttsText}
      placeholder="Texto para sintetizar..."
      disabled={ttsBusy}
    ></textarea>
    <button onclick={speak} disabled={ttsBusy || !ttsText.trim()}>
      {ttsBusy ? "Sintetizando..." : "Falar"}
    </button>
    {#if ttsResult}
      <p class="counts">
        {ttsResult.voice} | audio {ttsResult.audio_secs.toFixed(2)} s |
        total {ttsResult.processing_ms} ms |
        1o byte ~{ttsResult.first_output_ms === null ? "n/d" : `${ttsResult.first_output_ms} ms`} |
        RTF {ttsResult.rtf.toFixed(2)} | {ttsResult.sample_rate} Hz
      </p>
    {/if}
    {#if audioUrl}
      <!-- Spikes tocam o WAV aqui; playback em Rust (rodio/cpal) e a tarefa M0-T08. -->
      <audio controls src={audioUrl}></audio>
    {/if}
    {#if ttsError}<p class="error">{ttsError}</p>{/if}
  </section>
  <section class="playback">
    <h3>Playback em Rust (M0-T08)</h3>
    <label>
      Backend:
      <select bind:value={pbBackend} disabled={pbBusy}>
        <option value="rodio">Rodio</option>
        <option value="cpal">Cpal</option>
      </select>
    </label>
    <button onclick={playRust} disabled={pbBusy || !ttsResult}>Tocar via Rust</button>
    <button onclick={stopRust} disabled={pbBusy}>Parar</button>
    <button onclick={conflictTest} disabled={pbBusy || !ttsResult}>
      {pbBusy ? "Aguarde..." : "Teste de conflito in+out"}
    </button>
    {#if !ttsResult}<p class="counts">Gere uma fala com "Falar" primeiro.</p>{/if}
    {#if pbInfo}
      <p class="counts">
        {pbInfo.backend} | wav {pbInfo.wav_secs.toFixed(2)} s | start ~{pbInfo.start_ms} ms
        (1o callback: ver log; so cpal)
      </p>
    {/if}
    {#if pbStop}
      <p class="counts">
        Stop {pbStop.backend}: <strong>{pbStop.stop_latency_ms} ms</strong> |
        drenou: {pbStop.drained ? "sim" : "nao"}
        {#if pbStop.samples_undrained !== null}| samples nao tocados: {pbStop.samples_undrained}{/if}
      </p>
    {/if}
    {#if conflict}
      <p class="counts">
        Conflito ({conflict.backend}): captura {conflict.capture_ok ? "OK" : "FALHOU"} |
        playback {conflict.playback_ok ? "OK" : "FALHOU"} |
        sobreposicao {conflict.overlap_secs.toFixed(1)} s
        {#if conflict.stop_report}| stop {conflict.stop_report.stop_latency_ms} ms{/if}
      </p>
      {#if conflict.capture_error}<p class="error">captura: {conflict.capture_error}</p>{/if}
      {#if conflict.playback_error}<p class="error">playback: {conflict.playback_error}</p>{/if}
    {/if}
    {#if pbError}<p class="error">{pbError}</p>{/if}
  </section>

  <section class="orbctl">
    <h3>Janela/Orb (M0-T09)</h3>
    <p>
      fps:
      {#each [60, 30, 15] as f}
        <button onclick={() => setOrbFps(f)} disabled={orbFps === f}>{f}</button>
      {/each}
      <button onclick={toggleOrbPause}>{orbPaused ? "Retomar RAF" : "Pausar RAF"}</button>
    </p>
    <p>
      estado fake:
      <select value={orbFakeState} onchange={(e) => setOrbFakeState(e.currentTarget.value)}>
        {#each ["Idle", "Listening", "Thinking", "Speaking", "Error"] as s}
          <option value={s}>{s}</option>
        {/each}
      </select>
    </p>
    <p>
      <button onclick={toggleClickThrough}>
        Click-through: {orbClickThrough ? "LIGADO (desligar)" : "desligado (ligar)"}
      </button>
    </p>
    <p>
      <button onclick={toggleOrbVisible}>{orbVisible ? "Ocultar orb (hide)" : "Mostrar orb"}</button>
      <button onclick={minimizeOrb}>Minimizar</button>
      <button onclick={restoreOrb}>Restaurar</button>
    </p>
    <p class="counts">
      alvo: {orbFps} fps{orbPaused ? " (RAF pausado)" : ""} |
      {#if orbStats}
        medido: {orbStats.fps} fps | rodando: {orbStats.running ? "sim" : "nao"} | document.hidden:
        {orbStats.hidden ? "sim" : "nao"}
      {:else}
        sem telemetria do orb ainda
      {/if}
    </p>
    {#if orbError}<p class="error">{orbError}</p>{/if}
  </section>
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
  .stt {
    text-align: center;
    font-size: 0.95rem;
    max-width: 90%;
  }
  .stt-text {
    margin: 0.5rem 0 0.25rem;
  }
  .tts {
    text-align: center;
    font-size: 0.95rem;
    max-width: 90%;
  }
  .playback {
    text-align: center;
    font-size: 0.95rem;
    max-width: 90%;
  }
  .playback select {
    background: #1e1e2a;
    color: #e6e6ee;
    border: 1px solid #33334a;
    border-radius: 4px;
    padding: 0.15rem 0.3rem;
  }
  .orbctl {
    text-align: center;
    font-size: 0.95rem;
    max-width: 90%;
  }
  .orbctl p {
    margin: 0.35rem 0;
  }
  .orbctl button,
  .orbctl select {
    background: #1e1e2a;
    color: #e6e6ee;
    border: 1px solid #33334a;
    border-radius: 4px;
    padding: 0.15rem 0.5rem;
  }
  .tts h3,
  .playback h3,
  .orbctl h3 {
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
    opacity: 0.8;
  }
  .tts textarea {
    width: 100%;
    box-sizing: border-box;
    background: #1e1e2a;
    color: #e6e6ee;
    border: 1px solid #33334a;
    border-radius: 6px;
    padding: 0.5rem;
    font: inherit;
    margin-bottom: 0.5rem;
  }
  .error {
    color: #ff6b6b;
    font-size: 0.85rem;
  }
</style>
