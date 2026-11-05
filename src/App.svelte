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
  .tts h3 {
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
