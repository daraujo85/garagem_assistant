# GaragemAssistant: brainstorming crítico da arquitetura

Contexto: o repo é greenfield (só o `.md` e `.jev/`). Não existe `.claude/jev-profile.md`, então não há arquitetura ou code style prévios a respeitar. Tudo abaixo vale para um projeto novo. Plataforma-alvo não está declarada. O path `/mnt/c` sugere Windows/WSL2, e isso muda várias decisões (ver seção 5).

---

## 1. Pontos fortes

- **Push-to-talk sem wake-word.** Elimina o modelo de wake-word, o consumo de CPU em repouso e metade dos problemas de privacidade. É a decisão certa.
- **Rust no core, UI como casca fina.** O orquestrador fica fora do webview, então o pipeline não depende do ciclo de vida da UI.
- **Separação de módulos** (`core/`, `audio/`, `db/`). Coerente e fácil de testar isoladamente.
- **SQLite como memória local.** Zero infra, portátil, backup trivial.
- **minijinja para personalidade.** Prompt parametrizado é melhor que strings coladas.
- **Piper para TTS.** Rápido, offline, boa qualidade/CPU.
- **`mcps.json` declarativo.** Bom ponto de partida para um registro de ferramentas explícito.

---

## 2. Riscos técnicos e gaps

### 2.1 Concorrência Rust/Tokio com áudio realtime

- **`cpal::Stream` é `!Send`.** Não dá para guardá-lo em `tauri::State` nem movê-lo entre tasks Tokio. Solução: uma thread OS dedicada de áudio (dona do `Stream`), comandada por canal (`Start`/`Stop`/`Shutdown`). O doc não menciona isso.
- **O callback do cpal não pode alocar, bloquear nem fazer `await`.** O `ringbuf` (SPSC lock-free) ajuda, mas o consumidor (resample, VAD, acumulação) deve rodar em thread própria ou `spawn_blocking`, nunca numa task async comum.
- **Mistura de canais.** O `Cargo.toml` lista `crossbeam-channel` e Tokio. Defina a regra: crossbeam ou `ringbuf` entre threads de áudio, `tokio::sync::mpsc`/`watch`/`broadcast` na fronteira async. Nada de `recv()` bloqueante dentro de `async fn`.
- **`async-process` é redundante.** `tokio::process` já cobre isso, e misturar runtimes de reactor causa dor. Remova.
- **Inferência bloqueante.** Whisper.cpp e Piper são CPU-bound. Execute em `spawn_blocking` ou num pool dedicado, com cancelamento (o usuário aperta o atalho de novo, ou "interrompe"). Sem um `CancellationToken` por turno, o pipeline vira spaghetti.
- **Barge-in ausente.** O que acontece se o usuário aperta o atalho enquanto o TTS fala? Deve parar o playback e iniciar nova captura. Isso exige uma máquina de estados explícita.
- **Sample rate e formato.** O mic costuma ser 44.1/48 kHz estéreo f32. O Whisper quer 16 kHz mono. O Piper emite 22.05 kHz tipicamente. Falta um estágio de resample (`rubato`) e normalização. O `rodio` abre device próprio, e dois streams (cpal in + rodio out) podem brigar em alguns drivers/WASAPI exclusivo.
- **Estado do turno.** Modele uma FSM única: `Idle → Listening → Transcribing → Thinking → Speaking → Idle` (+ `Error`, `Cancelled`). Mantenha em um único ator (task dona do estado) que recebe eventos (`ShortcutPressed`, `ShortcutReleased`, `SttDone`, `LlmDone`, `TtsChunk`, `Cancel`). Evita `Arc<Mutex<...>>` espalhado.
- **Key-up do atalho.** No Tauri 2, global-shortcut é plugin separado (`tauri-plugin-global-shortcut`), não `features = ["global-shortcut"]` no crate `tauri`. O `Cargo.toml` do doc está errado nisso. O evento `ShortcutState::Released` existe, mas é pouco confiável em algumas plataformas. No Linux/Wayland global shortcuts praticamente não funcionam sem portal. Faça um spike antes de assumir.

### 2.2 UI WebGL × estado async

- **Orb a 60 fps com picos de áudio.** Não mande evento Tauri por frame de áudio. `emit` serializa JSON via IPC, e a 60+ Hz isso satura e gera jitter. Alternativas:
  - Emitir níveis (RMS/FFT reduzido a 8–16 bandas) a ~30 Hz, e interpolar no frontend.
  - Ou usar `tauri::ipc::Channel` (streaming tipado, mais barato que `emit`).
- **Fonte de verdade única.** O estado vive no Rust. O frontend só renderiza (`state` + `level`). Não replique lógica de transição na UI.
- **Janela transparente/always-on-top/click-through.** É o formato natural de orb flutuante. WebView2 com transparência e GPU tem bugs e custo de memória (~100–200 MB). Verifique cedo.
- **Three.js é exagero para uma bolinha.** Um fragment shader em canvas puro (regl, ou WebGL cru, ~100 linhas) tem bundle e init muito menores. Three.js só se o orb crescer.
- **"~0% CPU em repouso" com WebGL animado.** Um orb pulsando no standby mantém GPU/compositor ativos. Reduza para 15–30 fps em Idle ou pause o RAF quando a janela está oculta. O claim de privacidade/consumo no doc é otimista.
- **Reconexão/recarga da UI.** Se o webview recarregar (dev HMR), a UI precisa pedir o snapshot do estado atual (`get_state`) e não depender só de eventos.

### 2.3 STT: Whisper.cpp vs "LFM3"

- **LFM3 não é um STT.** No seu ambiente, `ollamac/lfm3` (LFM3-1.7B) é um LLM de texto. Confirme se existe uma variante de áudio (a família Liquid tem modelos de áudio específicos). Se não, a linha "Whisper.cpp / LFM3 local" mistura categorias e deve ser reescrita como "Whisper.cpp (STT) e, separadamente, LFM3 como LLM local de fallback".
- **Whisper.cpp:** `whisper-rs` (bindings) ou `whisper.cpp` como sidecar. Prefira `whisper-rs` linkado com `small`/`base` quantizado (q5_1). Aceleração: CUDA/Vulkan/CoreML/Metal conforme SO. Sem GPU, `base.en`/`small` int8 levam ~0.5–2 s para 5 s de áudio num CPU moderno.
- **Alternativas a avaliar:** `faster-whisper` (CTranslate2, via sidecar Python, mais complexo), `sherpa-onnx` (Rust-friendly, streaming, Parakeet/Zipformer) e Moonshine (pequeno, rápido, EN). Para PT-BR, Whisper `small` ou `medium` é o realista. Moonshine é inglês.
- **Whisper não é streaming nativo.** Transcrever só depois de soltar a tecla soma a duração da transcrição à latência. Mitigação: transcrever em janelas durante a fala (chunks de 2–3 s com overlap) e finalizar rápido no release. Ou `sherpa-onnx` streaming.
- **Alucinação em silêncio.** O Whisper inventa texto ("Obrigado por assistir") em áudio vazio. Precisa de VAD (Silero via `ort`, ou `webrtc-vad`) e descarte de capturas curtas (<300 ms) ou sem voz.
- **Idioma.** PT-BR + termos técnicos em inglês (nomes de função, comandos). Defina `language=pt` forçado vs auto-detect, e considere `initial_prompt` com vocabulário dev.

### 2.4 Latência STT → LLM → TTS

Orçamento de referência (usuário solta a tecla até o primeiro som):

| Etapa | Alvo | Nota |
|---|---|---|
| Fim de captura + VAD | <50 ms | |
| STT | 300–800 ms | depende de modelo/HW e janelas |
| Roteamento/intenção | <100 ms | regra/heurística antes de LLM |
| LLM first token | 300–1500 ms | cloud via 9router; local 1.7B mais rápido |
| TTS first chunk | <200 ms | Piper por frase |
| **Total** | **~1–2.5 s** | |

Gaps:
- **Sem streaming no meio.** O doc descreve pipeline em lotes. O ganho vem de: LLM em stream de tokens, quebrar por sentença (`.?!\n`) e mandar cada frase pro Piper assim que fecha, tocando enquanto o resto gera. Isso corta a latência percebida pela metade.
- **Dois LLM hops.** "Jev dispatcher" (intenção) + "LLM de resposta" dobra o custo. Faça a classificação de intenção com regras/embeddings/modelo pequeno local ou na mesma chamada (tool calling nativo), não numa chamada extra.
- **Respostas de ack.** Para ações longas (MCP), toque um "ok, já vejo" pré-gravado/curto imediatamente.
- **Medir desde o dia 1.** Instrumente cada estágio com `tracing` spans e salve p50/p95 localmente. Sem isso, não dá para otimizar.
- **Cold start.** Carregar Whisper/Piper no primeiro uso custa segundos. Pré-carregue no boot (ou em lazy-warmup) e mantenha residente, medindo RAM (~500 MB–1.5 GB).

### 2.5 Schema SQLite

- **Migrations versionadas.** `sqlx::migrate!` com arquivos `NNNN_nome.sql` já resolve. O doc cita a pasta mas o schema está num bloco solto. Defina `0001_init.sql` e nunca edite migrations aplicadas.
- **Índices ausentes.** Mínimo: `UNIQUE(category, key)` em `memories`, índice em `(category, last_accessed DESC)`, índice em `mcp_registry(name)` ou `UNIQUE(name)`.
- **Tipos.** `DATETIME DEFAULT CURRENT_TIMESTAMP` no SQLite vira TEXT em UTC. Padronize em `INTEGER` (unix ms) ou ISO-8601 e escolha um formato só. `BOOLEAN` é INTEGER 0/1. Use `STRICT` tables e `CHECK`.
- **IDs TEXT.** Use UUIDv7/ULID (ordenável) ou INTEGER autoincrement. UUIDv4 em PK TEXT fragmenta e não ordena.
- **`preferences` como KV de strings.** `humor` é "0–100" guardado como texto, sem validação. Valide no Rust (struct tipada com serde) e use `CHECK`.
- **"RAG local" sem vetores.** `memories` só tem `key`/`value` texto. Não há embedding nem FTS. Opções incrementais:
  1. FTS5 (`memories_fts`) com `bm25`. Zero dependência, já ajuda muito.
  2. `sqlite-vec` para embeddings (bge-m3 já roda no seu Ollama na rede).
  Comece com FTS5, deixe vetor para depois.
- **Faltam tabelas:** `conversations`/`turns` (histórico, necessário para contexto multi-turno e debug), `tool_calls` (auditoria de MCP), `settings` de áudio. Considere também `memories.source`, `created_at`, `expires_at`, `pinned`.
- **Categoria `'secret'` em texto puro.** Gravar segredos (tokens, senhas) em SQLite sem criptografia é risco. Opções: guardar no keychain do SO (`keyring` crate) e só uma referência no DB, ou SQLCipher. E **nunca injetar `secret` em `{{ injected_memories }}`** que vai para LLM cloud.
- **Política de esquecimento.** `access_count`/`last_accessed` existem, mas não há quem os use. Defina scoring (recência × frequência × relevância) e limite de tokens injetados.
- **Config do SQLite:** WAL, `foreign_keys=ON`, `busy_timeout`, `synchronous=NORMAL`. Pool pequeno.

### 2.6 `mcp_discovery.rs`: segurança

Este é o maior risco do projeto. "Varredura de executáveis" + LLM + comandos por voz = execução remota controlada por prompt injection.

- **Superfície:** varrer `PATH`/diretórios e registrar qualquer binário como ferramenta dá ao LLM capacidade de executar qualquer coisa. Voz ambiente, texto em uma página/arquivo lido por uma tool, ou saída de outro MCP podem injetar instruções ("rode `curl ... | sh`").
- **Princípio:** descoberta ≠ habilitação. Descobrir só *sugere*. Habilitar exige ação humana explícita na UI (o toggle de MCPs já existe no `SettingsPanel`, mantenha como padrão `is_active = 0`).
- **Allowlist, não scan livre.** Fontes confiáveis e fechadas: manifestos MCP conhecidos (`.mcp.json`, config do Claude Desktop/Cursor), diretório próprio `~/.garagem/tools/`, e lista curada de CLIs (git, gh, rg...). Evite varrer `PATH` inteiro.
- **Pin de identidade:** guarde caminho canônico + hash SHA-256 do binário e valide antes de cada execução. Se mudou, desativa e pede reaprovação (evita TOCTOU/binary swap).
- **Execução sem shell.** `Command::new(path).args([...])`, nunca `sh -c` com string do LLM. Valide argumentos por JSON Schema da tool.
- **Capabilities por tool:** classifique em `read`, `write`, `network`, `destructive`. Confirmação obrigatória (falada + clique/atalho) para `write`/`destructive`. Nunca confirmação só por voz para destrutivo.
- **Sandbox/limites:** timeout, limite de saída, env limpo (sem herdar tokens: filtre `AWS_*`, `*_KEY`), cwd restrito, e quando possível sandbox do SO (Job Objects/AppContainer no Windows, bubblewrap/landlock no Linux).
- **Log de auditoria** de todo tool call (tabela `tool_calls`: quem pediu, args, exit code, hash). Revisável na UI.
- **Dados de ferramenta são não confiáveis.** Marque saída de tools como contexto `untrusted` no prompt e nunca deixe ela alterar `preferences`/`memories` sem passar por regra explícita.
- **Transporte HTTP no `mcp_registry`:** SSRF/localhost. Restrinja hosts, exija TLS ou loopback explícito.
- **Mapeie para um cliente MCP real.** Há SDK Rust oficial (`rmcp`). Evite implementar o protocolo à mão. `command_path TEXT` sozinho não guarda args/env/cwd. Falta `args_json`, `env_json`, `capabilities`, `hash`, `approved_at`.

### 2.7 Fallback de rede e áudio sensível

- **Áudio bruto nunca sai da máquina.** STT é local, então só *texto* seguiria para cloud. Isso precisa ser invariante de arquitetura (teste/assert), não convenção. O diagrama diz "STT local", mas o fallback para OpenAI/Ollama remoto poderia ser lido como enviar áudio. Deixe explícito.
- **Mas o texto também é sensível.** Transcrição pode conter segredo falado, nome de cliente, código proprietário. Defina níveis:
  - `local-only`: só modelos locais (LFM3/Ollama local). Sem rede.
  - `cloud-allowed`: via 9router, com redação.
  - Por turno e por categoria de memória: memórias `secret`/`project_context` marcados como `no_egress`.
- **Redação antes de egress:** regex/detectores para chaves (AKIA…, ghp_…, JWT), emails, IPs. Passar por filtro único antes de qualquer chamada de rede.
- **Fallback silencioso é perigoso.** Cair de local para cloud por timeout sem avisar quebra a promessa de "privacidade total" do doc. Indicador visível (cor da orb ou ícone) e política configurável: pergunta/bloqueia/permite.
- **Ollama "remoto".** No seu ambiente, Ollama roda em LAN (`192.168.31.58`, Mac M4). É remoto do ponto de vista do app e trafega HTTP sem TLS. Trate como egress (rede confiável, mas não local).
- **Claim de "privacidade total"** no topo do doc é incompatível com fallback OpenAI. Reescreva como "áudio nunca sai; texto só sai conforme política".
- **Chaves de API:** keychain do SO, não `preferences` nem `mcps.json`.

### 2.8 Erros no pipeline Jev + Nine-router

- **Definições ambíguas.** O doc não explica o papel de cada um. Pelo seu ecossistema: 9router é gateway HTTP (`localhost:20128`) com combos e fallback próprio. Jev é a engine (`jev-claude-engine`). Decida:
  - Jev = dispatcher in-process (Rust) ou processo externo (Node, `bin/jev.js`)? Se for Node, o "app local Rust leve" ganha dependência de runtime Node e IPC.
  - Quem faz fallback: 9router (combos) ou o app? Fallback em dois níveis (9router + app → Ollama/OpenAI) gera retry-storm e latência multiplicada. Escolha um dono.
- **Taxonomia de erros** (enum `thiserror`): `SttFailed`, `SttEmpty`, `LlmTimeout`, `LlmRateLimited`, `GatewayDown`, `ToolDenied`, `ToolFailed`, `TtsFailed`, `AudioDeviceLost`. Cada um com ação definida: retry, fallback, ou fala de erro curta ("não consegui falar com o modelo").
- **Timeouts por estágio + deadline global** do turno. Sem isso, a orb fica "Pensando" para sempre.
- **Gateway indisponível:** health check periódico leve; degradação para LFM3 local via `ollamac` ou modo "só transcrever e mostrar texto".
- **Erros audíveis e visíveis.** A UI precisa de estado `Error` (cor própria), e o TTS deve poder falar uma mensagem de erro pré-gravada (não depender do pipeline que acabou de falhar).
- **Retry idempotente.** Retentar um turno com tool call `write` pode duplicar efeito. Separe chamadas de LLM (retry livre) de execução de tool (sem retry automático).
- **Observabilidade:** `tracing` + arquivo rotativo local com `turn_id` correlacionando STT/LLM/tool/TTS. Sem logar texto de transcrição por padrão (privacidade).

### 2.9 Outros gaps

- **Prompt injection via memória:** `{{ injected_memories }}` sem escape/delimitação. Use delimitadores explícitos e instrução de "dados, não comandos". `minijinja` com autoescape não protege contra isso (é injeção semântica, não de sintaxe).
- **Template de personalidade com "palavrão liberado"** e "sarcástico": é só prompt. Para `explicit_allowed=false`, TTS/saída não garante nada. Aceitável para uso pessoal, só deixe documentado como best-effort.
- **Piper:** projeto original foi arquivado/migrado (rhasspy/piper → OHF-Piper/piper1-gpl, licença GPL). Verifique a licença se for distribuir. Voz PT-BR existe (faber, edresson) mas qualidade média. Considere Kokoro/Supertonic como alternativa a testar. Integre via `piper` binário como sidecar ou `piper-rs`/ONNX com `ort`.
- **Empacotamento e modelos:** modelos (centenas de MB a GB) não vão no instalador. Precisa de downloader com checksum, pasta de modelos, e first-run wizard.
- **Permissão de microfone** (macOS TCC, Windows privacy settings) e comportamento quando o device muda (headset plugado).
- **Versões desatualizadas no Cargo.toml:** `sqlx 0.7` → 0.8; `rodio 0.17` → 0.19/0.20; `cpal 0.15` ok; `ringbuf 0.3` → 0.4; `tokio 1.36` → atual. Fixe após `cargo add`, não copie.
- **Testes:** nenhuma estratégia. Defina traits (`Stt`, `Tts`, `Llm`, `ToolRunner`, `AudioSource`) com impls fake para testar a FSM sem hardware.

---

## 3. Decisões de stack questionáveis e alternativas

| Decisão | Avaliação | Recomendação |
|---|---|---|
| **Vue vs Svelte** | UI minúscula (orb + settings). Diferença de produtividade irrelevante. Svelte 5 gera bundle menor e runes simplificam estado reativo vindo de eventos. Vue é melhor se Diego já domina. | Escolha o que você já domina. Se nenhum, **Svelte** (menos runtime). Não deixe "(ou Svelte)" no doc: decida. |
| **Three.js** | Pesado para uma esfera animada. | Shader GLSL em canvas puro ou `ogl`/`regl`. Three só se virar cena 3D. |
| **sqlx vs rusqlite** | sqlx exige runtime async, macros `query!` precisam de DB em build/`sqlx prepare` (atrito em CI), e SQLite não tem I/O async real (usa thread pool interno). `rusqlite` é síncrono, simples, rápido, com `rusqlite_migration` ou `refinery`, e permite FTS5/`sqlite-vec` direto. | Para app desktop single-user: **rusqlite + 1 thread/`spawn_blocking`** (ou `tokio-rusqlite`) é mais simples. sqlx só se você valoriza checagem de queries em compile time e migrações embutidas. Ambos viáveis, mas decida conscientemente. |
| **minijinja vs tera** | minijinja: leve, runtime sem dependências pesadas, sintaxe Jinja2, bom para prompts. Tera: mais features (herança, filtros), mais deps. | **Manter minijinja.** Ressalva: carregue templates de arquivo (`include_str!` + override em disco) para editar personalidade sem recompilar. |
| **cpal + ringbuf + rodio** | `rodio` abre seu próprio output stream. Mixar com cpal in tem edge cases. | Considerar usar só `cpal` (in e out) com um ring buffer de saída, controlando a fila TTS e o barge-in diretamente. Mais controle de latência e cancelamento. `rodio` serve para o MVP. |
| **crossbeam-channel + tokio** | Dois sistemas de canais. | Regra clara por fronteira (ver 2.1). |
| **async-process** | Redundante. | `tokio::process`. |
| **Whisper.cpp "/" LFM3** | Categorias diferentes. | Decidir STT = whisper-rs; LFM3 = LLM local. |
| **Piper** | Boa latência, licença/manutenção a checar, PT-BR mediano. | Manter para MVP, abstrair atrás de trait `Tts` para trocar. |
| **Jev + 9router + fallback Ollama/OpenAI** | Camadas sobrepostas. | Um cliente HTTP OpenAI-compatível apontando para o 9router (`localhost:20128`) com combo escolhido (ex.: `claude-tudao`/`claude-coder`), e o fallback local (`ollamac/*`) configurado no próprio 9router. O app só fala com um endpoint. |
| **Tauri 2 vs alternativas** | Boa escolha. WebView2/WebKitGTK varia. Para orb simples poderia ser egui/wgpu nativo (sem webview, menos RAM, WebGL→wgpu). | Manter Tauri se settings UI for relevante. Considerar `wgpu` nativo só se RAM/CPU do webview incomodar. |
| **Template de personalidade só com `humor`** | Eixo único. | OK para MVP. Evolua para perfis nomeados (`profiles`) depois. |

---

## 4. Melhorias incrementais e ordem de implementação

### Princípio
Entregue primeiro o **loop mínimo ponta a ponta com tudo falso/simples**, depois substitua cada peça. Não comece por `mcp_discovery` nem por WebGL.

### Fase 0: Spikes de risco (1–3 dias, descartáveis)
1. Global shortcut com **press e release** no SO-alvo (Tauri 2 plugin). Se Released não for confiável, o design inteiro de PTT muda.
2. `cpal` capturando mic em thread dedicada → WAV 16 kHz mono (com `rubato`).
3. `whisper-rs` transcrevendo esse WAV em PT-BR, medindo tempo (base/small, CPU vs GPU).
4. Piper gerando PCM e tocando. Medir tempo até o primeiro áudio.
5. Janela transparente, always-on-top, com canvas animado. Medir CPU/RAM em idle.

Saída: tabela de latências reais e decisão go/no-go por componente.

### Fase 1: MVP "fala → texto → LLM → fala" (sem memória, sem MCP)
- FSM única (`Idle/Listening/Transcribing/Thinking/Speaking/Error`) num ator Tokio; traits `Stt`, `Llm`, `Tts`, `AudioIn`, `AudioOut`.
- Cliente LLM OpenAI-compatível → 9router, com streaming SSE.
- Streaming por sentença LLM → Piper → playback.
- Orb com 4 estados + nível de áudio (30 Hz via `Channel`).
- `tracing` com spans por estágio e `turn_id`.
- Cancelamento e barge-in.
- Template minijinja fixo + `preferences` via arquivo TOML (ainda sem SQLite).

Critério de pronto: p50 < 2 s da soltura da tecla ao primeiro som, orb sincronizada, cancelar funciona.

### Fase 2: Persistência e personalidade
- SQLite com migrations (`0001_init`, `UNIQUE`s, índices, STRICT, WAL).
- `preferences` tipadas + `SettingsPanel` (humor, explicit, nome, atalho, modo de privacidade).
- Tabela `turns` (histórico) e contexto multi-turno com janela de tokens.
- Keychain para chaves de API.

### Fase 3: Memória
- Comandos de voz "lembra que...", "esquece...": escrita explícita, nunca automática no começo.
- FTS5 para recuperação, com limite de tokens e delimitadores anti-injeção.
- Marcação `no_egress` e redação de egress.
- Só depois, avaliar embeddings (`sqlite-vec` + bge-m3).

### Fase 4: Ferramentas (MCP)
- Começar com **allowlist estática** em `mcps.json` (2–3 tools de leitura: git status, rg, ler arquivo no projeto).
- Cliente MCP via `rmcp`, execução sem shell, timeout, env limpo, auditoria `tool_calls`.
- Confirmação na UI para `write`/`destructive`.
- Só então: *descoberta* como sugestão (nunca auto-ativa), com hash pin e aprovação.

### Fase 5: Robustez e privacidade
- Política de egress (local-only/cloud), indicador na UI, modo degradado quando o gateway cai.
- VAD (Silero), janelas de STT parcial, tuning de latência.
- Taxonomia de erros completa, mensagens de erro faladas pré-gravadas.
- Downloader de modelos + first-run wizard, empacotamento, auto-update.

### Fase 6: Extras
- Perfis de personalidade, wake via múltiplos atalhos, TTS alternativo, embeddings, sandbox do SO para tools.

---

## 5. Perguntas em aberto para o Diego

**Plataforma e escopo**
1. **SO-alvo?** Windows nativo, Linux (qual DE/Wayland?), macOS? O path `/mnt/c` sugere Windows/WSL2. WSL2 não é boa plataforma para áudio/atalhos globais/Tauri GUI. O app deve ser compilado e rodar no Windows host?
2. É projeto pessoal ou será distribuído? Isso define licenças (Piper GPL), empacotamento, auto-update e rigor de segurança.
3. Idioma: PT-BR, EN, ou misto (com termos técnicos em EN)?

**IA/Pipeline**
4. **O que é "LFM3" aqui?** LLM local de fallback ou STT? Existe variante de áudio que você pretende usar?
5. **Quem é o "Jev" no runtime?** Código Rust in-process, ou `jev-claude-engine` (Node) como sidecar? Qual o contrato (HTTP, stdio, CLI)?
6. **Quem faz fallback:** 9router (combos) ou o app? Qual combo padrão (`claude-tudao`, `claude-coder`)?
7. **Meta de latência** aceitável (1 s? 2 s? 3 s)? E aceita rodar STT com GPU? Que hardware é o alvo mínimo?
8. **Conversa multi-turno** ou comando único por acionamento? Quanto histórico manter?
9. Haverá **resposta apenas por voz**, ou também painel de texto (útil para código/saídas de tools)? TTS não serve bem para ler código.

**Privacidade e segurança**
10. **Política de egress:** o texto transcrito pode ir para cloud por padrão? Há categorias (segredos, projetos de clientes) que nunca podem?
11. Fallback para OpenAI deve ser **silencioso, perguntar, ou bloqueado**?
12. **Nível de autonomia das tools:** o assistente pode escrever/executar sem confirmação? Quais ações exigem confirmação (e por qual canal)?
13. A descoberta de MCP deve **só sugerir** ou pode **ativar automaticamente**? (Recomendo só sugerir.)
14. `category = 'secret'` é para armazenar segredos de verdade? Se sim, keychain/SQLCipher é obrigatório.

**Dados e stack**
15. **sqlx ou rusqlite?** Valoriza macros de compile-time ou simplicidade?
16. **Vue ou Svelte?** Qual você já domina?
17. Memória precisa de **busca semântica** (embeddings) desde cedo, ou FTS5 basta?
18. Sincronização de memória entre máquinas (Mac/Deepin/Duck) é requisito? Se sim, SQLite local puro não basta, e isso é decisão arquitetural grande (agentmemory já existe no seu ambiente: reaproveitar em vez de reinventar `memories`?).
19. **Reuso do agentmemory (`:3111`)** como backend de memória em vez de tabela própria? Reduz escopo, mas cria dependência de serviço externo e quebra o "100% local/offline".

**Produto**
20. Qual é o **primeiro caso de uso concreto** que define "funcionou"? (ex.: "qual o status do git?", "abre o ticket X", "lembra que...")? Isso prioriza quais tools e evita construir descoberta genérica cedo demais.

---

## 6. Resumo executivo

- **Manter:** Tauri+Rust, PTT, SQLite, minijinja, módulos separados.
- **Corrigir já no doc:** global-shortcut é plugin; remover `async-process`; LFM3 não é STT; claim de "privacidade total" vs fallback cloud; versões de crates; schema (índices, STRICT, tipos, `UNIQUE(category,key)`, tabelas `turns`/`tool_calls`).
- **Maior risco técnico:** áudio `!Send` + FSM + cancelamento + latência (resolver por spike e arquitetura de ator).
- **Maior risco de segurança:** `mcp_discovery` com varredura livre. Trocar por allowlist + aprovação humana + hash pin + sem shell.
- **Ordem:** spikes → loop falso ponta a ponta com streaming por sentença → persistência → memória FTS5 → tools allowlist → descoberta como sugestão → egress policy/robustez.
- **Decisões que travam o início:** SO-alvo, papel do Jev/9router/LFM3, política de egress, sqlx vs rusqlite, Vue vs Svelte.
