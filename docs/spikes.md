# Spikes (M0)

Registro dos spikes de risco do marco M0 (ver `docs/plano.md`). Codigo de spike e descartavel; as medicoes ficam aqui.

## Spike PTT (M0-T03)

**Status:** implementado, NAO compilado nem testado (escrito em WSL2 sem Rust). Medicoes pendentes no Windows.

### O que foi implementado

- `src-tauri/src/shortcut.rs` (novo): handler do `tauri-plugin-global-shortcut` para o atalho `Ctrl+Shift+Space` (constante `PTT_SHORTCUT`, hardcoded; M2: via preferences).
  - `Pressed`: guarda `Instant::now()` em `PttState` (`Mutex<...>` gerenciado via `tauri::State`), loga via `tracing::info!` (`t_ms` desde o inicio do app, resolucao de `Instant`) e emite o evento Tauri `shortcut-pressed`.
  - `Released`: emite `shortcut-released` com `hold_ms` (Released menos Pressed) e `repeats`, e loga os mesmos dados.
  - `Pressed` repetido durante um hold (repeticao de tecla do SO) e contado em `repeats` e ignorado: nao reinicia o cronometro nem reemite o evento.
  - `Released` sem `Pressed` correspondente gera `warn!` (evento perdido; relevante pro go/no-go).
  - O `Instant` e capturado na primeira linha do handler, para o jitter nao incluir log/emit.
- `src-tauri/src/main.rs`: registra o plugin com handler (`shortcut::plugin()`), gerencia `PttState` e registra o atalho no `setup`. Falha de registro (atalho em uso) e logada com `error!`, sem derrubar o app.
- `src/App.svelte`: `listen()` dos dois eventos; mostra estado do PTT (ocioso/escutando), duracao do ultimo hold em ms, repeticoes do SO e contadores de Pressed/Released. Tambem faz `console.log` de cada evento (devtools). O orb fica verde (Listening) enquanto o atalho esta pressionado.
- Fora de escopo (M0-T05/M1): audio, thread dedicada do `cpal::Stream` (`!Send`), FSM.

### Como testar no Windows

1. Na raiz do repo, no PowerShell: `npm install` e `npm run tauri dev`.
2. Na janela do app, abrir o devtools (clique direito > Inspecionar, ou `Ctrl+Shift+I`) e ir na aba Console.
3. Observar o stdout do processo Rust (o terminal do `npm run tauri dev`). Para mais detalhe: `$env:RUST_LOG="info"` antes de rodar. Procurar as linhas `atalho global registrado`, `shortcut Pressed` e `shortcut Released` (com `t_ms` e `hold_ms`).
4. Para cada cenario da tabela abaixo, segurar `Ctrl+Shift+Space` e soltar. Conferir:
   - a tela mostra "escutando" ao pressionar e "ocioso" ao soltar, e os contadores Pressed/Released sobem juntos;
   - o log Rust e o console do devtools mostram ambos os eventos;
   - o `hold_ms` bate com o tempo real do hold (cronometre com um timer externo para estimar o jitter de release).
5. Jitter: comparar `hold_ms` com a duracao real; para vários holds curtos (ex. 10 x ~300 ms) anotar a variacao. Anotar tambem qualquer `Released sem Pressed` no log.
6. Se aparecer `falha ao registrar atalho global`, outro app usa a combinacao: trocar `PTT_SHORTCUT` e repetir.

### Resultados (preencher no Windows)

Ambiente: SO/versao: ____ | Teclado/layout: ____ | Versao de `tauri-plugin-global-shortcut`: ____

| Cenario | Pressed disparou? | Released disparou? | Jitter observado (ms) | Observacoes |
|---|---|---|---|---|
| App em foco | | | | |
| App em segundo plano / outra janela em foco | | | | |
| Tela cheia de outro app | | | | |
| Repeticao de tecla do SO (hold longo) | | | | Ver `repeats` no log: quantos Pressed extras chegaram? Released unico no fim? |
| Dois toques rapidos seguidos | | | | Algum par Pressed/Released perdido? Contadores iguais? |

### Criterio de go/no-go

- **GO** se Pressed e Released disparam de forma confiavel (sem perda de eventos) em pelo menos os 2 primeiros cenarios.
- **NO-GO** caso contrario. Nesse caso, avaliar M0-T04 (plano B: hook de baixo nivel ou modo toggle).

**Decisao:** ____ (data: ____)

## Spike Captura de Audio (M0-T05)

**Status:** implementado, NAO compilado nem testado (escrito em WSL2 sem Rust). Medicoes pendentes no Windows.

### O que foi implementado

- `src-tauri/src/audio/capture.rs` (substitui o stub): `AudioCapture::start()` / `.stop()` (ver brainstorming 2.1 e plano M0-T05).
  - **Thread OS dedicada** (`audio-capture`) dona do `cpal::Stream` (`!Send`), comandada por `crossbeam-channel` com `Start`/`Stop`/`Shutdown`. `AudioCapture` so guarda o sender e o `JoinHandle`, entao e `Send + Sync` e pode morar em `tauri::State`. `Drop` envia `Shutdown` e junta a thread.
  - **Callback do cpal sem alocacao e sem bloqueio:** converte o formato nativo (f32/i16/u16/i32) para f32 num buffer fixo de stack e faz `push_slice` num `ringbuf` SPSC (`HeapRb`, 4 s de folga). Se encher, descarta e incrementa um contador atomico (`dropped_samples`), sem nunca esperar.
  - **Consumidor em thread propria** (`audio-consumer`): le o ringbuf (polling de 5 ms), faz downmix para mono (media simples dos canais), resample para 16 kHz com `rubato::FftFixedIn` (chunks de 1024 frames; cauda final com `process_partial`) e acumula em `Vec<f32>`. Device ja em 16 kHz pula o resample.
  - **No `Stop`:** derruba o stream, drena o ringbuf, junta o consumidor e grava `./captures/capture_<unix_ms>.wav` (16-bit PCM, 16 kHz, mono, via `hound`). A pasta e criada se nao existir. `start_in(dir)` permite outro diretorio.
  - **Logs (`tracing`):** device (nome, sample rate nativo, canais, formato), inicio, e no fim `wall_s` (relogio play->stop), `native_s` (frames recebidos / taxa nativa), `wav_s` (samples do WAV / 16000), contagem de frames/samples, `dropped_samples`, `peak` e `rms`. Warns para ringbuf estourado e audio silencioso.
  - Erros tipados com `thiserror` (`AudioCaptureError`: sem device, config, formato nao suportado, stream, resampler, I/O, WAV, etc.).
- `src-tauri/src/shortcut.rs`: `AudioState` (`Mutex<Option<AudioCapture>>`, similar ao `PttState`). No `Pressed` (primeiro, sem repeticao) chama `AudioCapture::start()`; no `Released` chama `.stop()` e loga o caminho do WAV (`stop_ms` e `hold_ms`).
- `src-tauri/src/main.rs`: `.manage(shortcut::AudioState::default())`.
- `src-tauri/Cargo.toml`: dependencias `rubato = "0.16"` e `hound = "3.5"`.
- `.gitignore`: `captures/` e `*.wav` (tambem evita o watcher do `tauri dev` reiniciar o app ao gravar o WAV).
- Fora de escopo (M1/M5): VAD, deteccao de silencio, FSM, escolha de device por preferences.

### Como testar no Windows

1. Na raiz do repo, no PowerShell: `$env:RUST_LOG="info"` e `npm run tauri dev`.
2. Segurar `Ctrl+Shift+Space` falando algo por ~3 s e soltar.
3. No terminal, conferir os logs `device de entrada escolhido` (nome, sample rate, canais), `captura iniciada` e `captura finalizada` (`wall_s`, `native_s`, `wav_s`, `dropped_samples`, `peak`).
4. Conferir o arquivo em `src-tauri/captures/` (o cwd do `tauri dev` e `src-tauri/`; o caminho completo esta no log `WAV gerado`).
5. Tocar o WAV e confirmar que o audio esta correto: sem distorcao, sem velocidade errada (acelerado/lento), sem corte no inicio/fim.
6. Verificar formato do WAV (ex.: propriedades do arquivo ou `ffprobe`): 16 kHz, mono, 16-bit.
7. Repetir para cada device/configuracao da tabela (mic a 44,1 kHz e 48 kHz, mono e estereo; trocar em Configuracoes de Som do Windows > Formato do dispositivo de entrada, e reiniciar o app, pois o device padrao e lido a cada `Start`).

### Resultados (preencher no Windows)

Ambiente: SO/versao: ____ | Versoes de `cpal` / `rubato` / `hound` resolvidas: ____

| Device | Sample rate nativo | Canais | WAV gerado (sim/nao) | Duracao esperada vs duracao do WAV | Observacoes |
|---|---|---|---|---|---|
| | | | | | |
| | | | | | |
| | | | | | |
| | | | | | |

Observacoes a anotar: `start_ms` (custo de abrir o device no Pressed), `dropped_samples` (deve ser 0), `peak`/`rms`, e se algum formato (i16/i32) exigiu o caminho de conversao.

### Criterio de go/no-go

- **GO** se o WAV gerado tem a duracao esperada (+-10%), 16 kHz mono, e o audio soa sem distorcao/aceleracao/corte.
- **NO-GO** caso contrario: revisar o resample ou o buffer sizing.

**Decisao:** ____ (data: ____)

---

## Spike STT (M0-T06)

Objetivo (plano M0-T06): `whisper-rs` transcrevendo o WAV da captura em PT-BR, com modelos `base` e `small` quantizados (q5_1), medindo tempo por 5 s de audio (CPU e, se houver, GPU), RAM residente, efeito do `initial_prompt` com vocabulario dev e taxa de erro em frases de referencia com termos tecnicos.

### O que foi implementado

- `src-tauri/src/audio/stt.rs` (novo):
  - `WhisperStt::load(path)`: carrega o modelo ggml uma vez (`WhisperContext::new_with_params`). Erro claro, com link de download, se o arquivo nao existir. Caminho padrao `./models/ggml-base.bin`, sobrescrevivel pela env `GARAGEM_STT_MODEL`.
  - `WhisperStt::transcribe(&Path) -> Result<TranscriptionResult, SttError>`: le o WAV (`hound`, exige 16 kHz mono, 16-bit ou f32), converte para f32 e roda `state.full` com `language="pt"` forcado, `initial_prompt` com vocabulario dev (git, commit, branch, merge, pull request, funcao, variavel, deploy, build, terminal, codigo...), Greedy (`best_of: 1`), sem contexto anterior e `n_threads = min(cores, 8)`.
  - `TranscriptionResult`: texto, idioma (forcado), duracao do audio, `processing_ms`, `rtf`, segmentos com timestamps (ms), nome do modelo e caminho do WAV. `processing_ms` cobre so a inferencia (nao o carregamento do modelo nem a leitura do WAV); o tempo de carga sai no log `modelo Whisper carregado` (`load_ms`).
  - Log via `tracing` de duracao, tempo, RTF e texto (spike: logar o texto e aceitavel para validar qualidade; producao M1+ nao loga transcricao por padrao).
  - Comando Tauri `transcribe_last_capture`: pega o WAV mais recente (mtime) de `./captures/` e transcreve em `tokio::task::spawn_blocking` (brainstorming 2.1). O modelo e carregado de forma lazy na primeira chamada (`SttState`) e fica residente.
- `src-tauri/src/main.rs`: `.manage(audio::stt::SttState::default())` e o comando no `invoke_handler`.
- `src/App.svelte`: botao "Transcrever ultima captura" que mostra texto, modelo, duracao, tempo de processamento, RTF e idioma.
- `src-tauri/Cargo.toml`: `whisper-rs = "0.13"` e feature opcional `cuda = ["whisper-rs/cuda"]`.
- Fora de escopo (M1/M5): VAD, streaming por janelas, descarte de silencio, pre-load no boot, FSM.

### Modelos (download manual)

Nada e baixado pelo app. Fonte: <https://huggingface.co/ggerganov/whisper.cpp/tree/main> (mesmo conteudo do repo `ggerganov/whisper.cpp`, script `models/download-ggml-model.sh`).

| Modelo | Arquivo | Tamanho aprox. |
|---|---|---|
| base | `ggml-base.bin` (f16), `ggml-base-q5_1.bin` | ~142 MB / ~60 MB |
| small | `ggml-small.bin` (f16), `ggml-small-q5_1.bin` | ~466 MB / ~190 MB |

Os nomes `-q5_1` devem ser conferidos na listagem do repo (os tamanhos sao aproximados). Colocar em `src-tauri/models/` (o cwd do `tauri dev` e `src-tauri/`; a pasta `models/` e `*.bin` ja estao no `.gitignore`). Para escolher o modelo: `$env:GARAGEM_STT_MODEL="./models/ggml-small-q5_1.bin"` antes do `npm run tauri dev`. Modelos `.en` nao servem (PT-BR exige o multilingue).

### Frases de teste sugeridas (PT-BR com termos tecnicos)

1. "faz um commit dessa branch e abre um pull request"
2. "roda o build e verifica os logs do terminal"
3. "qual e a funcao que trata esse erro"
4. "da um merge da branch main na minha branch e resolve os conflitos"
5. "cria uma variavel nova para guardar o resultado do deploy"
6. "refatora essa funcao e adiciona um teste unitario"
7. "mostra o diff do ultimo commit"
8. "reinicia o servidor e confere se o build passou"
9. "por que o deploy falhou ontem a noite"
10. "abre o terminal e roda o git status"

Gravar cada frase com ~5 s (e uma captura de silencio puro, para observar alucinacao; brainstorming 2.3).

### Como testar no Windows

1. Instalar os pre-requisitos de build do `whisper-rs`: CMake, MSVC Build Tools e LLVM (`LIBCLANG_PATH`). Para GPU, CUDA Toolkit e `--features cuda`.
2. Baixar os modelos (secao acima) para `src-tauri/models/`.
3. `$env:RUST_LOG="info"` e `npm run tauri dev` (primeiro build do whisper.cpp e demorado). Se a API do `whisper-rs` divergir, ajustar `stt.rs` (ver o aviso no topo do arquivo).
4. Gravar uma frase com o atalho `Ctrl+Shift+Space` (spike M0-T05) e soltar.
5. Clicar "Transcrever ultima captura". A primeira chamada inclui a carga do modelo (`load_ms` no log); repetir o clique para medir o tempo "quente".
6. Anotar na tabela: duracao, `processing_ms`, RTF, texto obtido e erros. Medir a RAM residente do processo no Gerenciador de Tarefas apos a carga.
7. Repetir para `base` e `small` (f16 e q5_1), em CPU e, se houver GPU, com `--features cuda`. Comparar tambem com e sem `initial_prompt` (editar `INITIAL_PROMPT` em `stt.rs`).

### Resultados (preencher no Windows)

Ambiente: SO/versao: ____ | CPU: ____ | GPU: ____ | Versao resolvida de `whisper-rs`: ____ | RAM residente apos carga (base / small): ____

| Modelo | Duracao audio (s) | Tempo processamento (ms) | RTF (tempo_proc / duracao_audio) | Texto esperado | Texto obtido | Erros de transcricao | CPU ou GPU |
|---|---|---|---|---|---|---|---|
| | | | | | | | |

### Criterio de go/no-go

- **GO** se RTF < 1.0 em CPU com o modelo `base` ou `small`, e taxa de erro aceitavel nos termos tecnicos (ver brainstorming 2.4: orcamento de 300-800 ms para 5 s de audio).
- **NO-GO** caso contrario: considerar modelo menor, GPU obrigatoria, ou alternativas como `sherpa-onnx` (ver brainstorming 2.3).

**Decisao:** ____ (data: ____)
## Spike TTS (M0-T07)

Objetivo (plano M0-T07): Piper (binario sidecar) sintetizando PT-BR, medindo tempo ate o primeiro audio e ate o fim, por frase curta e por frase longa, comparando as vozes `faber` e `edresson` (brainstorming 2.9: vozes PT-BR existentes, qualidade mediana). Registrando a licenca (risco R-M0-3).

### O que foi implementado

- **Piper como processo externo (sidecar), nao binding Rust.** Nao ha binding Rust maduro (ao contrario do `whisper-rs`/whisper.cpp); `std::process`/`tokio::process` invocando o binario `piper` e a integracao mais robusta (brainstorming 2.9).
- `src-tauri/src/audio/tts_piper.rs` (substitui o stub):
  - `PiperTts::new(bin, voice)`: valida binario + voz (`<voz>.onnx` E `<voz>.onnx.json`); erro claro com onde baixar se faltar. `PiperTts::from_env()` le `GARAGEM_PIPER_BIN` (default `./tools/piper/piper.exe` no Windows) e `GARAGEM_PIPER_VOICE` (default `./models/voices/pt_BR-faber-medium.onnx`).
  - `PiperTts::synthesize(&str)`: spawna `piper --model <voz>.onnx --output_file <saida>.wav`, escreve o texto no stdin (terminado em `\n`: uma linha = uma utterance), fecha o stdin (EOF dispara a sintese), espera o processo terminar e le a duracao no header do WAV com `hound`.
  - Metricas em `SynthesisResult`: `processing_ms` (spawn ate o processo terminar, INCLUI a carga do modelo ONNX que no sidecar acontece a cada chamada), `first_output_ms` (APROXIMACAO por polling do arquivo a cada 5 ms), `audio_secs`, `sample_rate`, `rtf`, voz usada e caminho do WAV.
  - `TtsError` (`thiserror`): `BinaryNotFound`, `VoiceNotFound`, `EmptyText`, `ProcessFailed` (com cauda do stderr), `Timeout` (60 s, mata o processo), `Io`, `Wav`. `kill_on_drop` evita processo orfao no timeout; stderr lido em task propria (pipe cheio = deadlock).
  - Comando Tauri `synthesize_speech(text)`: `tokio::process::Command` com `await` (ESCOLHA DOCUMENTADA: o trabalho pesado roda no processo Piper, nao ha CPU-bound dentro do Rust, entao nao precisa de `spawn_blocking` — brainstorming 2.1). Comando `read_synthesized_wav(path)`: devolve os BYTES do WAV (`tauri::ipc::Response` binario, chega como `ArrayBuffer`) com checagem de que o caminho esta dentro de `./captures/tts/` (nao vira leitor generico de disco para o webview).
  - WAVs de saida em `./captures/tts/` (a pasta `captures/` ja esta no `.gitignore` e fora do watcher do `tauri dev`; `latest_wav` do stt.rs nao e recursivo, entao nao colide com o spike STT).
- `src/App.svelte`: campo de texto + botao "Falar" -> `synthesize_speech` -> blob URL -> tag `<audio>` HTML (playback em Rust e M0-T08, tarefa separada).
- `src-tauri/src/main.rs`: comandos `synthesize_speech` e `read_synthesized_wav` no `invoke_handler`.
- **Fora de escopo (M1):** streaming de PCM (`--output-raw`), processo Piper residente, fila de sintese/playback, quebra por frase.

### Download manual (nada e baixado pelo app)

**Binario do Piper (Windows):**

| Fonte | Licenca | Observacao |
|---|---|---|
| <https://github.com/rhasspy/piper/releases> (`piper_windows_amd64.zip`) | MIT | Projeto original, arquivado; e o que as flags `--model`/`--output_file` deste spike assumem |
| <https://github.com/OHF-Voice/piper1-gpl> (fork ativo) | **GPL** | Fork ativo; flags podem diferir (`--output-file`/`-f`); ajustar `build_args` em `tts_piper.rs` se usar este |

Extrair em `src-tauri/tools/piper/` (pasta `tools/` no `.gitignore`).

**Vozes PT-BR** (`.onnx` + `.onnx.json` — OS DOIS arquivos, o `.json` e obrigatorio):

- <https://huggingface.co/rhasspy/piper-voices> (caminho `pt/pt_BR/<nome>/<qualidade>/`). Vozes comuns: `pt_BR-faber-medium` (padrao do spike) e `pt_BR-edresson-low`. Qualidade mediana (brainstorming 2.9).
- Colocar em `src-tauri/models/voices/` (pasta `models/` e `*.onnx` no `.gitignore`).

**Licenca (risco R-M0-3):** o Piper original (rhasspy) e MIT; o fork ativo (OHF-Piper/piper1-gpl) e GPL. Isso SO importa se houver distribuicao — decisao D2 do plano esta "Nao" (uso pessoal) mas segue em aberto ate M5/M6. **Decisao pendente:** se D2 mudar para distribuido, trocar para o binario MIT do rhasspy ou reavaliar alternativa (Kokoro/Supertonic, brainstorming 3). Nao linkar o binario do Piper no instalador enquanto D2 estiver aberta.

### Frases de teste (PT-BR)

Curtas:

1. "Olá, como posso ajudar?"
2. "Qual o status do git?"
3. "Já anotei."
4. "Ok, já vejo."

Longas (2-3 linhas):

5. "Fiz o merge da sua branch na main e o build passou sem erros. Os testes unitarios novos tambem rodaram. Quer que eu abra o pull request agora?"
6. "Lembrei que voce prefere commits pequenos com mensagens em portugues. O ultimo commit quebrei os testes de integracao, entao reverti e deixei anotado no log para voce revisar depois do almoço."

### Como testar no Windows

1. Baixar o binario do Piper para `src-tauri/tools/piper/` e as vozes para `src-tauri/models/voices/` (secao acima; OS arquivos, `.onnx` + `.onnx.json`).
2. Se usar o fork piper1-gpl e as flags divergirem, editar `build_args` em `tts_piper.rs`.
3. `npm run tauri dev`.
4. Digitar uma frase, clicar "Falar", ouvir o resultado pela tag `<audio>` e anotar: `total` (processing_ms), `1o byte` (aproximado), duracao do audio, RTF e a qualidade percebida (1-5). A PRIMEIRA chamada pode ser mais lenta (disco frio / antivirus); medir a partir da segunda.
5. Comparar `pt_BR-faber-medium` com `pt_BR-edresson-low`: trocar a voz ou `$env:GARAGEM_PIPER_VOICE="./models/voices/pt_BR-edresson-low.onnx"` antes do `tauri dev`.
6. Repetir com as frases curtas e longas acima (e uma frase sua, para nocao subjetiva de uso diario).

### Resultados (preencher no Windows)

Ambiente: SO/versao: ____ | CPU: ____ | Versao do binario Piper: ____ | Fonte (rhasspy/piper1-gpl): ____

| Voz | Texto (curto/longo) | Tempo total (ms) | Duracao do audio gerado (s) | Qualidade percebida (1-5) | Observacoes |
|---|---|---|---|---|---|
| | | | | | |

### Criterio de go/no-go

- **GO** se tempo total < 500 ms para frases curtas (brainstorming 2.4: meta de <200 ms pro primeiro chunk, mas o spike mede arquivo completo, nao streaming) e qualidade de voz aceitavel para uso diario.
- **NO-GO** caso contrario: considerar outras vozes, ou streaming de PCM via `--output-raw` (adiado pro M1), ou TTS alternativo (Kokoro, Supertonic — ver brainstorming 3).

**Decisao:** ____ (data: ____)

---

## Spike Playback (M0-T08)

**Status:** implementado, NAO compilado nem testado (escrito em WSL2 sem Rust). Medicoes pendentes no Windows.

Objetivo (plano M0-T08, risco R-M0-5): responder com dados reais, no SO-alvo, (A) se `rodio` (out) e `cpal` (in) coexistem no mesmo device sem conflito de WASAPI, e (B) qual a latencia real de parar o playback (base do barge-in do M1: DoD exige parar o audio em < 200 ms). Decide o backend do `AudioOut` do M1-T10 (brainstorming 2.1 e 3: `cpal` puro in+out vs `rodio`).

### O que foi implementado

- `src-tauri/src/audio/playback.rs` (novo): `PlaybackHandle::play_wav(path, PlaybackBackend)`, `.stop() -> StopReport` (com `stop_latency_ms`) e `.is_playing()`. Thread OS dedicada (`audio-playback`) por playback, comandada por `crossbeam-channel` (`Stop`/`Shutdown`), mesma regra do `capture.rs` (o `cpal::Stream` e `!Send`). `Drop` envia `Shutdown` e junta a thread.
  - **Backend Rodio:** `rodio::OutputStream` + `Sink` (device proprio do rodio), WAV decodificado por streaming (`Decoder`). `stop()` = `sink.stop()` + poll de 1 ms ate `sink.empty()` (teto de 1 s).
  - **Backend Cpal:** WAV decodificado com `hound` (int 8/16/24/32 ou float; multicanal -> mono), resample com `rubato` para a taxa do device (o Piper emite 22,05 kHz), WAV inteiro colocado num `ringbuf` SPSC (`HeapRb`) ANTES do stream iniciar. O callback de saida so faz `pop_slice` (sem alocar/bloquear), expande mono -> N canais e escreve silencio no resto. `stop()` = seta flag atomica (o callback passa a escrever silencio no proximo periodo) + drop do `cpal::Stream`. Reporta tambem `samples_undrained` (o que sobrou no ringbuf).
  - **Metricas logadas (`tracing`) e retornadas:** duracao do WAV, `start_ms`, `first_sample_ms` (so cpal), `stop_latency_ms`, `drained`, `underruns` (cpal).
  - **Aproximacoes (importante ao ler os numeros):**
    - `start_ms` = `play_wav` ate o stream estar iniciado. NAO e o primeiro sample audivel: soma ~1 periodo do device (5-20 ms tipico).
    - `first_sample_ms` (cpal) = primeira invocacao do callback de saida = quando o device comecou a puxar samples. Melhor aproximacao observavel de dentro do Rust. Rodio: nao observavel (`None`) sem source customizada.
    - `stop_latency_ms` no **cpal** = pedido de stop ate o drop do stream. No **rodio** = pedido de stop ate `sink.empty()`. Nenhum dos dois ve o buffer interno do WASAPI (10-20 ms em modo compartilhado), entao ambos SUBESTIMAM o stop acustico; o do rodio tende a subestimar mais. Se os numeros ficarem perto do limite de 200 ms, confirmar de ouvido ou gravando a saida.
- Comandos Tauri (em `playback.rs`, registrados no `main.rs`): `play_wav_rust(path, backend)` (`"rodio"`/`"cpal"`), `stop_playback()` e `spike_conflict_test(path, backend)`. Estado `PlaybackState` (um playback por vez; novo play derruba o anterior). Operacoes bloqueantes via `spawn_blocking`.
- `src/App.svelte`: secao "Playback em Rust (M0-T08)" com seletor Rodio/Cpal, botoes "Tocar via Rust", "Parar" e "Teste de conflito in+out". A tag `<audio>` original do spike TTS foi mantida para comparacao (ela nao permite medir stop no Rust).
- `src-tauri/src/audio/mod.rs` e `main.rs`: modulo, `.manage(PlaybackState)` e comandos.
- Sem novas dependencias (`cpal`, `rodio`, `ringbuf`, `rubato`, `hound`, `crossbeam-channel` ja estavam no `Cargo.toml`).
- **ATENCAO — API do rodio:** a versao exata e a API (`OutputStream::try_default()` + `Sink::try_new` + `Decoder::new`) NAO foram verificadas. Confirmar no primeiro `cargo build` no Windows e ajustar `rodio_control`/`rodio_stop` se necessario.
- **Fora de escopo (M1):** FSM, barge-in integrado a logica do assistente, fila/streaming de TTS, niveis RMS para a UI.

### Como testar no Windows (item B: latencia de stop)

1. Garantir o TTS do M0-T07 funcionando (binario do Piper e voz). `$env:RUST_LOG="info"` e `npm run tauri dev`.
2. Digitar uma frase LONGA (a 5 ou 6 da secao TTS, >= 8 s de audio) e clicar "Falar". Isso gera o WAV em `src-tauri/captures/tts/`.
3. Selecionar **Rodio**, clicar "Tocar via Rust". Conferir nos logs: `device`, `start_ms`, duracao do WAV. Confirmar de ouvido que a velocidade/tom estao corretos.
4. Clicar "Parar" em momentos diferentes: logo no inicio (~0,5 s), no meio e perto do fim (~1 s antes de acabar). Anotar `stop_latency_ms` (tela e log) de cada tentativa. Repetir 5x por ponto, anotando min/mediana/max.
5. Repetir os passos 3-4 com **Cpal**. Anotar tambem `first_sample_ms`, `samples_undrained` e `underruns` (deve ser 0).
6. Comparar com a tag `<audio>` (so de ouvido: ela nao tem medicao).
7. Se possivel, validar o stop acustico de ouvido ou gravando saida e clique: `stop_latency_ms` e proxy, nao a medida acustica final.

### Teste de conflito in+out (item A)

**Automatico (botao "Teste de conflito in+out"):** inicia a captura (`AudioCapture::start`, mesmo caminho do PTT/M0-T05), toca o WAV por cima com o backend selecionado, espera a duracao do WAV + 0,5 s, para o playback (medindo o stop COM a captura ativa, cenario real de barge-in) e finaliza a captura, gravando o WAV. A UI mostra `captura OK/FALHOU`, `playback OK/FALHOU` e erros de device. Rodar uma vez por backend, falando algo durante o teste, e conferir se o WAV de captura (`src-tauri/captures/`) tem audio sem cortes/glitches. O comando "Falar" precisa ter sido usado antes (precisa de um WAV).

**Manual (alternativa / confirmacao, cobre a captura via atalho real):**
1. Selecionar o backend, gerar um WAV longo.
2. Segurar `Ctrl+Shift+Space` (captura ativa) e, ainda segurando, clicar "Tocar via Rust".
3. Ver nos logs se ha erro de device (`falha ao abrir o stream`, `AUDCLNT_E_DEVICE_IN_USE`, `exclusive`...), se o `capture` finalizou com `dropped_samples = 0` e se o playback tocou limpo.
4. Testar tambem com um app externo usando o mic/saida em modo exclusivo (Configuracoes de Som > Propriedades do dispositivo > Avancado > "Permitir que aplicativos assumam o controle exclusivo"), ligado e desligado, para provocar o conflito de WASAPI.

### Resultados (preencher no Windows)

Ambiente: SO/versao: ____ | Device de saida (nome/taxa/canais): ____ | Device de entrada: ____ | Versoes de `cpal` / `rodio` resolvidas: ____

| Backend | Playback inicia ok? | Stop latency (ms) | In+Out simultaneo funciona? | Observacoes (erros de device, glitches) |
|---|---|---|---|---|
| Rodio | | | | |
| Cpal | | | | |

Para o campo "Stop latency", anotar min/mediana/max e o ponto do audio onde foi clicado (inicio/meio/fim). Anotar tambem `start_ms`, `first_sample_ms` (cpal) e `underruns`.

### Criterio de go/no-go

- **GO** se o stop latency < 200 ms (alvo de barge-in do M1 DoD) em pelo menos um dos backends, E se o in+out simultaneo funciona (sem erro de WASAPI exclusivo) no mesmo backend. Isso decide qual backend o M1-T10 usa.
- Se os dois backends passarem, preferir o **cpal** (brainstorming 3: mais controle de latencia e de cancelamento, sem segundo device aberto), salvo se o rodio for claramente mais simples sem custo na latencia.
- **NO-GO** caso contrario: investigar buffer size do cpal/WASAPI, device unico compartilhado por in e out, ou outro backend de saida.

**Decisao:** ____ (data: ____)
