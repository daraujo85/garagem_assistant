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
