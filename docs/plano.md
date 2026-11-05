# GaragemAssistant: Plano de Projeto

Versão: 0.1 (rascunho). Data: 2026-11-05.

Base: `arquitetura_garagemassistant.md` e `docs/brainstorming.md` (seções 2, 4 e 5).
O repo é greenfield e não há `.claude/jev-profile.md`, então não existe arquitetura prévia a preservar. O plano estende o desenho do brainstorming.

## Resumo dos marcos

| Marco | Nome | Objetivo (1 linha) | Estimativa | Status |
|---|---|---|---|---|
| M0 | Spikes de risco | Validar PTT press/release, áudio, STT, TTS e janela transparente no SO-alvo com números reais e go/no-go por componente | 3-5 d | Não iniciado |
| M1 | Loop de voz ponta a ponta | Falar, transcrever, LLM em streaming, TTS por sentença e orb sincronizada, sem memória e sem tools | 10-14 d | Não iniciado |
| M2 | Persistência e personalidade | SQLite migrado, preferences tipadas, SettingsPanel, histórico de turnos multi-turno e segredos no keychain | 7-10 d | Não iniciado |
| M3 | Memória | Lembrar/esquecer por comando explícito, recuperação via FTS5 com limite de tokens e proteção anti-injeção | 6-8 d | Não iniciado |
| M4 | Ferramentas / MCP | Executar tools de allowlist estática via `rmcp`, de forma segura, com confirmação e auditoria | 10-14 d | Não iniciado |
| M5 | Robustez, privacidade e egress | Política de egress aplicada e visível, VAD, taxonomia de erros completa e modo degradado | 8-12 d | Não iniciado |
| M6 | Polish, empacotamento, extras | Instalador, downloader de modelos, first-run wizard, descoberta de MCP como sugestão e extras | 10-15 d | Não iniciado |

Total estimado: ~54-78 dias úteis, para 1 dev. As estimativas são grosseiras e devem ser recalibradas com os dados do M0.

---

## Premissas provisórias (PENDENTE DE CONFIRMAÇÃO DO USUÁRIO)

O usuário (Diego) ainda não respondeu as 20 perguntas da seção 5 do brainstorming. Onde não há decisão, o plano assume a opção recomendada no brainstorming como default provisório. Nada abaixo está decidido. Cada linha precisa de confirmação antes do marco que a consome.

| ID | Tema (pergunta do brainstorming) | Default provisório assumido | Bloqueia | Confirmado? |
|---|---|---|---|---|
| D1 | SO-alvo (Q1) | Windows nativo, com build e execução no host, não em WSL2 | M0 | Não |
| D2 | Pessoal vs distribuído (Q2) | Uso pessoal, sem distribuição pública | M5, M6 | Não |
| D3 | Idioma (Q3) | PT-BR com termos técnicos em EN, `language=pt` forçado e `initial_prompt` com vocabulário dev | M0, M1 | Não |
| D4 | O que é LFM3 (Q4) | LLM local de fallback (`ollamac/lfm3`), não STT | M1, M5 | Não |
| D5 | Papel do Jev (Q5) | Não é componente de runtime. O app fala com um único endpoint OpenAI-compatível | M1 | Não |
| D6 | Dono do fallback e combo (Q6) | 9router é dono do fallback (combo `claude-tudao`), com `ollamac/*` configurado no próprio 9router. O app não faz retry entre provedores | M1, M5 | Não |
| D7 | Meta de latência e HW (Q7) | p50 menor que 2 s. STT em CPU como baseline, GPU opcional. HW mínimo a definir no M0 | M0, M1 | Não |
| D8 | Multi-turno (Q8) | Multi-turno com janela de tokens. Comando único no M1 | M1, M2 | Não |
| D9 | Saída só voz vs painel de texto (Q9) | Voz mais painel de texto mínimo (transcrição e resposta) | M1, M4 | Não |
| D10 | Política de egress (Q10) | `cloud-allowed` com redação, e categorias `secret` e `project_context` como `no_egress` | M3, M5 | Não |
| D11 | Fallback silencioso (Q11) | Nunca silencioso: indicador visível, e política configurável com padrão "perguntar" | M5 | Não |
| D12 | Autonomia das tools (Q12) | `read` sem confirmação. `write` e `destructive` exigem confirmação na UI (clique ou atalho), nunca só por voz | M4 | Não |
| D13 | Descoberta de MCP (Q13) | Só sugere, nunca auto-ativa (`is_active = 0`) | M4, M6 | Não |
| D14 | `secret` armazena segredo real? (Q14) | Segredos no keychain do SO (`keyring`). O DB guarda só referência | M2, M3 | Não |
| D15 | sqlx vs rusqlite (Q15) | rusqlite + `rusqlite_migration` + `spawn_blocking` | M2 | Não |
| D16 | Vue vs Svelte (Q16) | Svelte 5. Vale a opção que o Diego já domina | M1 | Não |
| D17 | Busca semântica cedo? (Q17) | FTS5 apenas. Embeddings só no M6, se necessário | M3 | Não |
| D18 | Sync entre máquinas (Q18) | Não é requisito. SQLite local | M3, M6 | Não |
| D19 | Reuso do agentmemory (Q19) | Não reusar. Tabela própria, para manter o modo offline | M3 | Não |
| D20 | Primeiro caso de uso (Q20) | "qual o status do git?" (tool `read`) e "lembra que..." | M1 (critério de demo), M4 | Não |

Regra operacional: antes de iniciar cada marco, o responsável confirma ou corrige as decisões da coluna "Bloqueia". Se a confirmação não vier, o marco segue com o default e abre uma issue `decision-pending`. Nunca deixa a decisão sem dono.

---

## Ajustes em relação à estrutura sugerida

1. **Gate mínimo de egress no M1, política completa no M5.**
   - Toda chamada de rede do LLM passa por um único `LlmGateway`, mesmo sem política ativa.
   - Sem esse ponto único, o M3 (`no_egress`) e o M5 viram retrabalho disperso.
2. **Deadline global do turno e erros básicos no M1.**
   - Sem isso, a orb fica em "Pensando" para sempre.
   - A taxonomia completa e as mensagens faladas ficam no M5.
3. **Filtro anti-alucinação mínimo no M1.**
   - Descartar capturas com menos de 300 ms ou com RMS baixo.
   - VAD Silero e STT por janelas ficam no M5.
4. **Keychain no M2**, conforme o brainstorming, porque as chaves de API entram junto com settings.
5. **Downloader de modelos e first-run wizard no M6** (o brainstorming os colocava na Fase 5).
   - Eles dependem de SO e distribuição (D1, D2), e o M5 já é grande.
   - Durante M1-M5, os modelos ficam em pasta local configurável.
6. **M0 inclui scaffolding de repo e CI.** Sem isso, os spikes não têm onde viver e o M1 começa com atrito.
7. **Descoberta de MCP vai para o M6**, não para o M4.
   - O M4 entrega só allowlist estática.
   - O brainstorming diz "só então" descoberta, e ela é o maior risco de segurança.
   - Isolada no fim, pode ser cortada sem afetar o produto.

---

## Convenções do backlog

- **ID de tarefa:** `Mx-Tnn`. Cada uma vira uma issue do GitHub.
- **Labels sugeridas:**
  - Por marco: `m0-spikes` ... `m6-polish`.
  - Por área: `area:audio`, `area:core`, `area:ui`, `area:db`, `area:llm`, `area:tools`, `area:security`, `area:infra`.
  - Por tipo: `spike`, `decision-pending`, `risk`.
- **Estimativa:** S (~0,5 d), M (1 d), L (2 d). Uma tarefa maior que L deve ser dividida.
- **Marcos do GitHub:** um por marco (M0-M6), com a DoD copiada na descrição.
- **Convenção de blocos "DECISÃO NECESSÁRIA":** viram issues `decision-pending` atribuídas ao Diego e linkadas ao marco.

---

## M0: Spikes de risco / prova de conceito

**Objetivo:** reduzir incerteza técnica antes de investir no loop. Cada spike termina em go/no-go documentado.
Código descartável, exceto as medições.

**Dependências:** nenhuma. Início condicionado à confirmação de D1.

### DECISÃO NECESSÁRIA (bloqueia M0)
- **D1: SO-alvo.** Todos os spikes dependem disso. Rodar no SO errado invalida os números.
  - Windows nativo ou Linux: qual DE e se Wayland. Wayland quebra global shortcuts.
  - Assumido: Windows nativo, sem WSL2.
- **D3: Idioma.** Define o modelo Whisper e a voz Piper a testar. Assumido: PT-BR.
- **D7: Meta de latência e hardware mínimo.** Define o critério go/no-go do STT. Assumido: p50 menor que 2 s, CPU baseline.

### Tarefas
- [ ] **M0-T01 (S)** Criar o repo base: `cargo tauri init`, frontend escolhido (default Svelte 5, D16), `rustfmt`/`clippy`, `.gitignore`. Não commitar `graphify-out/` nem modelos.
- [ ] **M0-T02 (S)** CI mínimo (GitHub Actions no SO-alvo): `cargo check`, `clippy -D warnings`, `cargo test` e build do frontend.
- [ ] **M0-T03 (M)** **Spike PTT:**
  - `tauri-plugin-global-shortcut` (plugin separado, não feature do crate `tauri`).
  - Medir se `ShortcutState::Pressed` e `Released` chegam de forma confiável.
  - Testar com aplicativo em foco, em tela cheia e com repetição de tecla do SO.
  - Registrar jitter de release.
- [ ] **M0-T04 (M)** **Plano B de PTT**, só se T03 falhar:
  - Hook de baixo nível do SO (ex.: `rdev`/`SetWindowsHookEx`) ou modo toggle (aperta para começar, aperta para parar).
  - Documentar o impacto no design da FSM.
- [ ] **M0-T05 (L)** **Spike de captura:**
  - `cpal` em thread OS dedicada (o `Stream` é `!Send`), comandada por canal `Start`/`Stop`/`Shutdown`.
  - Callback sem alocação, alimentando `ringbuf`.
  - Consumidor em thread própria.
  - Resample com `rubato` para 16 kHz mono e gravação de WAV.
  - Testar mic a 44,1 kHz e 48 kHz, mono e estéreo.
- [ ] **M0-T06 (M)** **Spike STT:**
  - `whisper-rs` transcrevendo o WAV em PT-BR com `base` e `small` quantizados (q5_1).
  - Medir tempo por 5 s de áudio, em CPU e em GPU se houver (CUDA/Vulkan).
  - Medir RAM residente.
  - Testar `initial_prompt` com vocabulário dev.
  - Medir taxa de erro em 10 frases de referência com termos técnicos.
- [ ] **M0-T07 (M)** **Spike TTS:**
  - Piper (binário sidecar ou `ort`) gerando PCM PT-BR.
  - Medir tempo até o primeiro áudio e até o fim, por frase curta e por frase longa.
  - Comparar vozes `faber` e `edresson`.
  - Registrar a licença do Piper (GPL, ver risco R-M0-3).
- [ ] **M0-T08 (M)** **Spike de playback:**
  - Tocar o PCM com `rodio` e com `cpal` puro em simultâneo à captura.
  - Verificar conflito de device (WASAPI exclusivo).
  - Verificar a latência de parar o playback (base do barge-in).
- [ ] **M0-T09 (M)** **Spike de janela/orb:**
  - Janela transparente, always-on-top, sem borda, com canvas animado (shader cru).
  - Medir CPU, GPU e RAM do WebView2 em idle (60, 30 e 15 fps e RAF pausado).
  - Testar click-through.
- [ ] **M0-T10 (S)** **Spike de IPC:** `tauri::ipc::Channel` enviando 30 Hz de níveis (8-16 bandas) versus `emit` a 60 Hz. Medir jitter.
- [ ] **M0-T11 (S)** **Spike do gateway:**
  - Chamada SSE ao 9router (`localhost:20128`) com o combo padrão (D6).
  - Medir o primeiro token (p50/p95 em 20 chamadas).
  - Verificar o comportamento com o gateway fora do ar.
- [ ] **M0-T12 (M)** Consolidar a **tabela de latências reais** (STT, LLM first token, TTS first chunk, total estimado) em `docs/spikes.md`. Incluir go/no-go por componente, HW usado e versões de crate fixadas.

### Definition of Done
- [ ] Existe `docs/spikes.md` com tabela de medições reais por etapa (p50/p95) e a decisão go/no-go de cada um dos 5 componentes (PTT, captura, STT, TTS, janela).
- [ ] O PTT press/release funciona no SO-alvo, ou o plano B está escolhido e documentado.
- [ ] O orçamento de latência total estimado é menor ou igual à meta de D7, ou há uma decisão registrada de relaxar a meta ou trocar de componente.
- [ ] Idle da janela transparente: CPU menor que 2% e RAM menor que 250 MB no SO-alvo, ou plano B (fps reduzido, `wgpu`) registrado.
- [ ] Repo e CI verdes no SO-alvo.
- [ ] Versões de crates fixadas via `cargo add` (não copiadas do doc original).

### Riscos do marco
- **R-M0-1:** `Released` não confiável ou ausente no SO-alvo (brainstorming 2.1). Impacta o design inteiro do PTT.
- **R-M0-2:** WebView2 transparente com bug de GPU ou custo alto de RAM (2.2).
- **R-M0-3:** Piper migrou para `OHF-Piper/piper1-gpl`. A licença GPL só importa se houver distribuição (D2). A qualidade PT-BR é mediana (2.9).
- **R-M0-4:** Whisper `small` em CPU sem GPU pode estourar o orçamento de STT de 300-800 ms (2.3, 2.4).
- **R-M0-5:** Conflito `cpal` in com `rodio` out em WASAPI (2.1).

---

## M1: MVP loop de voz ponta a ponta (sem memória, sem tools)

**Objetivo:** segurar o atalho, falar, soltar e ouvir a resposta do LLM, com orb sincronizada, streaming por sentença, cancelamento e barge-in.
Tudo atrás de traits, para testar a FSM sem hardware.

**Dependências:** M0 concluído, com go/no-go de todos os componentes.

### DECISÃO NECESSÁRIA (bloqueia M1)
- **D5: Papel do Jev.** Assumido: o app fala com um endpoint OpenAI-compatível.
  - Se o Jev for um sidecar Node, o contrato (HTTP, stdio ou CLI) precisa existir antes da T05.
  - Se o Jev for um dispatcher in-process, o `dispatcher.rs` ganha escopo próprio.
- **D6: Dono do fallback e combo padrão.** Assumido: o 9router faz o fallback. Fallback duplo gera retry-storm.
- **D4: O que é LFM3.** Define se existe fallback local já no M1. Assumido: LLM local via 9router, tratado no M5.
- **D16: Vue ou Svelte.** Assumido: Svelte 5.
- **D8: Multi-turno.** Assumido: comando único no M1.
- **D9: Painel de texto.** Assumido: painel mínimo.
- **D20: Caso de uso de demo.** Define a frase usada no critério de aceite.

### Tarefas
- [ ] **M1-T01 (M)** Definir traits `AudioIn`, `AudioOut`, `Stt`, `Llm`, `Tts` e a camada de impls fake (áudio WAV fixo, STT e LLM com texto fixo, TTS silencioso). Estrutura de módulos conforme o doc de arquitetura, com `thiserror` desde o início.
- [ ] **M1-T02 (L)** **FSM em um ator Tokio:**
  - Estados `Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`, `Error` e `Cancelled`.
  - Eventos: `ShortcutPressed`, `ShortcutReleased`, `SttDone`, `LlmChunk`, `LlmDone`, `TtsChunk`, `Cancel`.
  - `CancellationToken` por turno e `turn_id`.
  - Sem `Arc<Mutex>` espalhado.
- [ ] **M1-T03 (M)** Testes unitários da FSM com os fakes: fluxo feliz, cancelar em cada estado, barge-in durante `Speaking`, evento inválido e deadline.
- [ ] **M1-T04 (L)** `AudioIn` real, a partir do spike T05 (thread dedicada, `ringbuf`, resample). Tratar perda do device com `AudioDeviceLost`.
- [ ] **M1-T05 (M)** `Stt` real com `whisper-rs` em `spawn_blocking`, com cancelamento cooperativo, `language` forçado (D3) e `initial_prompt`. O modelo é pré-carregado no boot (warmup).
- [ ] **M1-T06 (S)** Descarte mínimo anti-alucinação: captura menor que 300 ms ou RMS abaixo do limiar vira `SttEmpty` (sem chamar o LLM).
- [ ] **M1-T07 (L)** `LlmGateway`, o **único ponto de saída de rede do LLM**:
  - Cliente OpenAI-compatível com SSE para o 9router.
  - Timeout por estágio, cancelamento e *hook* de política de egress (no-op no M1).
  - Sem retry de provedor (D6).
- [ ] **M1-T08 (M)** Quebra de tokens em sentenças (`.?!\n`), com tratamento de abreviações e números e um limite de tamanho. Emite frases para o TTS assim que fecham.
- [ ] **M1-T09 (L)** `Tts` Piper (sidecar ou `ort`, conforme o spike) por sentença, com fila e saída PCM.
- [ ] **M1-T10 (L)** `AudioOut` com `cpal` único ou `rodio` (decidir pelo spike T08):
  - Fila de playback com `stop()` imediato, para o barge-in.
  - Emite níveis RMS para a UI.
- [ ] **M1-T11 (M)** Barge-in e cancelamento: o atalho durante `Speaking` para o playback, cancela o LLM e o TTS pendentes e inicia nova captura.
- [ ] **M1-T12 (M)** Prompt engine com `minijinja`: template fixo carregado via `include_str!`, com *override* em disco. `preferences` vindas de um arquivo TOML (ainda sem SQLite). Sem `injected_memories` por enquanto.
- [ ] **M1-T13 (M)** Plugin do atalho global integrado à FSM (a partir do spike T03 ou plano B T04). O atalho é configurável por arquivo.
- [ ] **M1-T14 (M)** Comandos Tauri `get_state` (snapshot, para reload da UI) e canal de eventos. O estado vive no Rust. A UI só renderiza `state` e `level`.
- [ ] **M1-T15 (L)** Orb com 4 estados mais `Error`. Shader em canvas cru, níveis a 30 Hz via `Channel`, fps reduzido em `Idle` e RAF pausado com a janela oculta.
- [ ] **M1-T16 (S)** Painel de texto mínimo (D9) com a transcrição e a resposta do último turno.
- [ ] **M1-T17 (M)** `tracing` com spans por estágio e `turn_id`, arquivo de log rotativo local. A transcrição **não** é logada por padrão. Salvar p50/p95 localmente.
- [ ] **M1-T18 (S)** Deadline global do turno e timeouts por estágio. Ao estourar, a FSM vai a `Error` e volta a `Idle`, sem orb presa.
- [ ] **M1-T19 (M)** Erros básicos audíveis: `SttEmpty`, `LlmTimeout`, `GatewayDown` e `TtsFailed`. As mensagens são WAVs pré-gravados ou sintetizados no build, sem depender do pipeline que falhou.
- [ ] **M1-T20 (M)** Medição ponta a ponta: script ou teste que mede soltura da tecla até o primeiro som em 20 turnos, e publica o resultado em `docs/latencia.md`.

### Definition of Done
- [ ] Demo reproduzível (caso de uso D20, ex.: pergunta simples falada) do atalho solto até a resposta falada, no SO-alvo, sem intervenção manual.
- [ ] **p50 menor que 2 s** da soltura da tecla ao primeiro som, e p95 menor que 3,5 s, em 20 turnos medidos (D7 pode ajustar).
- [ ] Cancelar funciona em todos os estados. Barge-in para o áudio em menos de 200 ms.
- [ ] O orb reflete o estado corretamente e o nível de áudio chega a ~30 Hz sem jitter visível. Idle com CPU menor que 2%.
- [ ] Nenhum turno deixa a orb presa em `Thinking`. O deadline força `Error` e volta a `Idle`.
- [ ] Cobertura de testes da FSM com os fakes, rodando no CI sem hardware.
- [ ] Todo egress de LLM passa por `LlmGateway` (verificado por busca no código ou por teste).
- [ ] Áudio bruto nunca é enviado pela rede. Há teste ou assert que impede o `LlmGateway` de aceitar bytes de áudio, só texto.

### Riscos do marco
- **R-M1-1:** `cpal::Stream` é `!Send`, e o callback não pode alocar nem bloquear (2.1). Mitigação: T04 com thread dedicada.
- **R-M1-2:** mistura de canais crossbeam e tokio (2.1). Mitigação: regra por fronteira, documentada em `docs/arquitetura.md`.
- **R-M1-3:** inferência bloqueante sem cancelamento vira spaghetti (2.1). Mitigação: `CancellationToken` por turno desde a T02.
- **R-M1-4:** alucinação do Whisper em silêncio (2.3). Mitigação parcial na T06 e completa no M5.
- **R-M1-5:** latência acima de 2 s por STT não-streaming (2.3, 2.4). Mitigação: streaming por sentença (T08) e, se necessário, antecipar STT por janelas do M5.
- **R-M1-6:** cold start de Whisper e Piper (2.4). Mitigação: warmup no boot.
- **R-M1-7:** dois LLM hops (dispatcher mais resposta) dobram o custo (2.4). Mitigação: sem dispatcher separado no M1. As tools do M4 entram via tool calling na mesma chamada.
- **R-M1-8:** UI sobrecarregada por eventos a 60 Hz (2.2). Mitigação: `Channel` a 30 Hz.

---

## M2: Persistência e personalidade

**Objetivo:** substituir o TOML por SQLite versionado, com preferências tipadas editáveis na UI, histórico de turnos para multi-turno e segredos fora do DB.

**Dependências:** M1 concluído (loop estável e `LlmGateway` único).

### DECISÃO NECESSÁRIA (bloqueia M2)
- **D15: sqlx ou rusqlite.** Assumido: rusqlite.
  - Com sqlx, o plano ganha tarefas de `sqlx prepare` e CI com DB.
  - É difícil trocar depois de escrever as queries.
- **D8: Multi-turno e quanto histórico manter.** Define o orçamento de tokens da janela.
- **D14: Segredos reais no keychain?** Assumido: sim, via `keyring`.
- **D10 (parcial):** a modelagem de `memories` e `turns` precisa saber se existe um nível `no_egress`. Assumido: sim.

### Tarefas
- [ ] **M2-T01 (M)** Infra de DB: `rusqlite` + `rusqlite_migration`, uma thread dona da conexão (ou `tokio-rusqlite`). PRAGMAs: WAL, `foreign_keys=ON`, `busy_timeout`, `synchronous=NORMAL`.
- [ ] **M2-T02 (L)** `0001_init.sql`:
  - Tabelas `STRICT`, ids UUIDv7 ou ULID, timestamps em `INTEGER` (unix ms), `CHECK`.
  - `preferences`, `memories` com `UNIQUE(category,key)` e índice `(category,last_accessed DESC)`.
  - Em `memories`: `source`, `created_at`, `expires_at`, `pinned` e `no_egress`.
  - `conversations`, `turns`.
  - Migration nunca é editada depois de aplicada.
- [ ] **M2-T03 (M)** `preferences` como struct tipada com `serde` (humor 0-100, `explicit_allowed`, `agent_name`, atalho, modo de privacidade) com validação no Rust e `CHECK` no DB.
- [ ] **M2-T04 (S)** Migrar o TOML do M1 para o DB, com importação única e fallback para defaults.
- [ ] **M2-T05 (M)** `prompt_engine` lendo `preferences` do DB a cada turno. Sem cache obsoleto: mudanças valem no próximo turno.
- [ ] **M2-T06 (M)** Comandos Tauri para ler e gravar preferences, com validação e evento de mudança.
- [ ] **M2-T07 (L)** `SettingsPanel` (humor, explicit, nome do agente, atalho, modo de privacidade). Regravar o atalho global ao alterá-lo.
- [ ] **M2-T08 (M)** Persistir turnos (`conversations`/`turns`): `turn_id`, texto do usuário, texto da resposta, latências por estágio. Registrar o texto só conforme a configuração de privacidade.
- [ ] **M2-T09 (M)** Contexto multi-turno com janela de tokens, descartando os mais antigos e com contador aproximado de tokens. Botão ou comando "nova conversa".
- [ ] **M2-T10 (M)** Keychain via `keyring`: chave de API do 9router e afins. O DB e o `mcps.json` nunca guardam segredo.
- [ ] **M2-T11 (S)** Teste de migrations: banco vazio até a última versão, e falha clara se a versão for mais nova que o binário.
- [ ] **M2-T12 (S)** Backup simples: cópia do DB antes de aplicar migration.

### Definition of Done
- [ ] Migrations aplicam do zero e em upgrade, em teste automatizado no CI.
- [ ] Alterar humor ou explicit no `SettingsPanel` muda o tom da resposta no turno seguinte, sem reiniciar.
- [ ] Valores inválidos (humor fora de 0-100) são rejeitados no Rust e no DB.
- [ ] O histórico multi-turno funciona em pelo menos 5 turnos encadeados, dentro do orçamento de tokens configurado, com o contexto cortado de forma previsível.
- [ ] Nenhuma chave de API em texto puro no DB, nos arquivos de config ou nos logs (verificado por grep no CI).
- [ ] Reiniciar o app preserva as preferências e o histórico.
- [ ] Latência p50 do M1 não regrediu mais que 10%.

### Riscos do marco
- **R-M2-1:** escolha de sqlx tem atrito em CI e macros `query!` (seção 3). Mitigação: default rusqlite (D15).
- **R-M2-2:** `preferences` KV de strings sem validação (2.5). Mitigação: T03.
- **R-M2-3:** tipos de data inconsistentes e PK TEXT fragmentada (2.5). Mitigação: T02 padroniza.
- **R-M2-4:** segredo em texto puro (2.5, 2.7). Mitigação: T10.
- **R-M2-5:** crescimento do contexto estoura tokens e latência. Mitigação: T09.
- **R-M2-6:** mudança de atalho em runtime pode deixar o shortcut antigo registrado. Mitigação: testar o ciclo de unregister e register.

---

## M3: Memória

**Objetivo:** "lembra que..." e "esquece..." por comando explícito, com recuperação por FTS5 injetada de forma segura no prompt.

**Dependências:** M2 (schema de `memories`, `turns`, keychain).

### DECISÃO NECESSÁRIA (bloqueia M3)
- **D17: Busca semântica desde cedo?** Assumido: FTS5 apenas, e embeddings no M6.
- **D19: Reuso do agentmemory (`:3111`)?** Assumido: não, tabela própria.
  - Reusar muda o M3 inteiro: vira um cliente HTTP, e perde o modo 100% offline.
  - Se for "sim", este marco precisa ser replanejado.
- **D18: Sync entre máquinas?** Assumido: não. Um "sim" torna o SQLite local insuficiente e é uma decisão arquitetural grande.
- **D10: Categorias que nunca saem da máquina** (`no_egress`). Define o filtro do T06.
- **D14: Como tratar `secret`.** Assumido: segredo real no keychain, e `memories` guarda só referência.

### Tarefas
- [ ] **M3-T01 (M)** `memory_store`: CRUD tipado (criar, ler, atualizar, apagar, listar por categoria). Upsert por `(category,key)`. `access_count` e `last_accessed` atualizados na leitura.
- [ ] **M3-T02 (M)** Migration `0002`: `memories_fts` (FTS5 `external content`) com triggers de sincronização e tokenizer adequado a PT (`unicode61 remove_diacritics`).
- [ ] **M3-T03 (L)** Comandos de voz explícitos "lembra que...", "esquece...", "o que você sabe sobre...":
  - Pelo tool calling do LLM ou por regra determinística antes do LLM. Avaliar no início da tarefa.
  - A escrita é sempre explícita, nunca automática.
  - O assistente confirma o que gravou ou apagou, falando.
- [ ] **M3-T04 (M)** Recuperação: consulta FTS5 com `bm25`, combinada com recência e frequência para o ranking, limite de N itens e de tokens.
- [ ] **M3-T05 (M)** Injeção anti-injection em `{{ injected_memories }}`:
  - Delimitadores explícitos.
  - Instrução "dados, não comandos".
  - Escape de delimitadores dentro do conteúdo.
  - Teste com memórias que contêm instruções maliciosas.
- [ ] **M3-T06 (M)** Marca `no_egress` e `secret`: memórias com essas marcas **nunca** entram em `injected_memories` enviado ao `LlmGateway`. Filtro no gateway (usa o hook do M1) e teste dedicado.
- [ ] **M3-T07 (M)** Detector de redação de egress, v1: regex para chaves (`AKIA...`, `ghp_...`, JWT), e-mails e IPs, aplicado também ao texto gravado em memória (avisar ao detectar um segredo).
- [ ] **M3-T08 (M)** UI de memória: listar, buscar, editar, apagar e fixar (`pinned`). Visualizar o que foi injetado no último turno.
- [ ] **M3-T09 (S)** Política de esquecimento: expiração (`expires_at`), limpeza em background e comando "esquece tudo sobre X".
- [ ] **M3-T10 (S)** Testes de recuperação com um conjunto de 30 memórias e 20 consultas em PT-BR. Medir precisão@3 e o custo de latência de FTS5.

### Definition of Done
- [ ] "Lembra que X" grava, "o que você sabe sobre X" recupera, e "esquece X" apaga, de ponta a ponta por voz, em teste manual e automatizado (com fakes).
- [ ] A recuperação FTS5 acrescenta menos que 50 ms ao turno, em DB com 10 mil memórias.
- [ ] Zero memórias `secret` ou `no_egress` aparecem em qualquer requisição do `LlmGateway` (teste que inspeciona o payload).
- [ ] Teste de injeção: uma memória com "ignore as instruções anteriores" não altera o comportamento (verificado em pelo menos 5 casos).
- [ ] Os tokens injetados respeitam o teto configurado.
- [ ] Nenhuma escrita automática em `memories` sem comando explícito.

### Riscos do marco
- **R-M3-1:** prompt injection via memória (2.9). Mitigação: T05.
- **R-M3-2:** segredo em texto puro e `secret` enviado à nuvem (2.5, 2.7). Mitigação: T06 e D14.
- **R-M3-3:** recuperação ruim em PT-BR (tokenização e acentos). Mitigação: T02 e T10. Embeddings ficam como plano B (M6).
- **R-M3-4:** comando de voz mal reconhecido apaga a memória errada. Mitigação: confirmação falada e visual antes de apagar mais de um item.
- **R-M3-5:** escopo cresce para sync (D18) ou agentmemory (D19). Mitigação: confirmação das decisões antes do início.

---

## M4: Ferramentas / MCP

**Objetivo:** o assistente executa 2-3 tools de leitura via MCP, com execução segura (sem shell, env limpo, timeout), confirmação para `write` e `destructive`, e auditoria em `tool_calls`.
Só allowlist estática. A descoberta fica para o M6.

**Dependências:** M3 concluído (ou ao menos M2). O `LlmGateway` e a política de confirmação da UI precisam existir.

### DECISÃO NECESSÁRIA (bloqueia M4)
- **D12: Autonomia das tools.** Quais ações exigem confirmação e por qual canal.
  - Assumido: `read` livre. `write` e `destructive` exigem clique ou atalho.
  - Confirmação só por voz nunca vale para destrutivo.
- **D20: Primeiro caso de uso concreto.** Define quais 2-3 tools entram. Assumido: git status, rg, ler arquivo dentro do projeto.
- **D13: Descoberta só sugere ou ativa?** Não bloqueia o M4, mas a modelagem do `mcp_registry` depende dela. Assumido: só sugere.
- **D9: Painel de texto.** Se a resposta da tool for código ou lista longa, só TTS não basta. Assumido: painel de texto.

### Tarefas
- [ ] **M4-T01 (M)** Migration `0003`: `mcp_registry` revisada (`UNIQUE(name)`, `args_json`, `env_json`, `cwd`, `capabilities`, `binary_hash`, `approved_at`, `is_active DEFAULT 0`) e `tool_calls` (turn_id, tool, args, exit code, hash, duração, resultado resumido).
- [ ] **M4-T02 (M)** Formato do `mcps.json` (allowlist estática) com schema validado. Carregamento no boot, com erro claro para entradas inválidas.
- [ ] **M4-T03 (L)** Cliente MCP via `rmcp` (SDK oficial), transporte stdio. Sem implementar o protocolo à mão. Ciclo de vida do processo (start, health, restart, shutdown).
- [ ] **M4-T04 (L)** Executor seguro:
  - `Command::new(path).args([...])`, nunca `sh -c`.
  - Env limpo (sem herdar `AWS_*` nem `*_KEY`).
  - `cwd` restrito, timeout e limite de saída.
  - Validação dos argumentos por JSON Schema da tool.
  - Sem retry automático de execução (idempotência).
- [ ] **M4-T05 (M)** Pin de identidade: caminho canônico mais SHA-256 do binário, validado antes de cada execução. Se mudou, desativa a tool e pede reaprovação.
- [ ] **M4-T06 (M)** Classificação de capabilities (`read`, `write`, `network`, `destructive`) por tool e política de confirmação em função dela.
- [ ] **M4-T07 (L)** Fluxo de confirmação na UI (modal e atalho), para `write` e `destructive`. A FSM ganha o estado `AwaitingConfirmation`, com timeout que resulta em negação.
- [ ] **M4-T08 (L)** Integração com o LLM por tool calling nativo na **mesma chamada** (sem 2º hop de LLM). O tool loop tem limite de iterações.
- [ ] **M4-T09 (M)** Saída de tool como contexto `untrusted`: delimitada no prompt e impedida de alterar `preferences` ou `memories` sem passar por regra explícita.
- [ ] **M4-T10 (S)** Ack falado ("já vejo...") para ações longas e indicador na orb de "executando tool".
- [ ] **M4-T11 (M)** Implementar as 2-3 tools do D20 (`git status`, `rg`, leitura de arquivo restrita a um diretório raiz configurável, com proteção contra path traversal).
- [ ] **M4-T12 (M)** UI de ferramentas: listar, ativar e desativar (toggle no `SettingsPanel`), ver o log de auditoria de `tool_calls`.
- [ ] **M4-T13 (M)** Testes de segurança: argumento com metacaracteres de shell, path traversal, binário trocado, saída gigante, timeout, tool que tenta ler env e saída contendo prompt injection.
- [ ] **M4-T14 (S)** Regras do transporte HTTP no registry: se previsto, restringir hosts, exigir TLS ou loopback explícito. Caso contrário, aceitar só stdio no M4 e documentar.

### Definition of Done
- [ ] Caso D20 funciona por voz: "qual o status do git?" executa a tool e responde corretamente, com o registro em `tool_calls`.
- [ ] Zero execuções via shell (verificado por busca no código e teste).
- [ ] Toda tool `write` ou `destructive` exige confirmação na UI. O teste prova que a voz sozinha não aprova.
- [ ] Binário alterado após a aprovação é bloqueado e a tool é desativada (teste automatizado).
- [ ] O env da tool não contém variáveis `*_KEY` nem `AWS_*` (teste).
- [ ] Tools nascem com `is_active = 0` e só ativam por ação do usuário.
- [ ] Todo tool call aparece no log de auditoria, visível na UI.
- [ ] A suíte de testes de segurança (T13) passa no CI.
- [ ] A latência adicional de uma tool de leitura simples (fora a própria execução) é menor que 300 ms.

### Riscos do marco
- **R-M4-1 (maior risco de segurança do projeto):** execução controlada por prompt injection via voz ambiente, arquivo lido ou saída de outra tool (2.6). Mitigação: allowlist, sem shell, pin, confirmação, saída `untrusted`.
- **R-M4-2:** TOCTOU e troca de binário (2.6). Mitigação: T05.
- **R-M4-3:** retry de turno duplica efeito de tool `write` (2.8). Mitigação: sem retry de execução (T04).
- **R-M4-4:** dois LLM hops dobram a latência (2.4). Mitigação: T08.
- **R-M4-5:** TTS não serve para ler código ou saídas longas (2.9, Q9). Mitigação: painel de texto.
- **R-M4-6:** `rmcp` pode ter lacunas de maturidade ou mudanças de API. Mitigação: isolar atrás de trait `ToolRunner` e fixar a versão.

---

## M5: Robustez, privacidade e política de egress

**Objetivo:** tornar a promessa de privacidade verificável e o app resiliente a falhas.
Entrega a política de egress configurável com indicador, o modo degradado, o VAD, a taxonomia de erros completa e o STT parcial.

**Dependências:** M4 (ou ao menos M3). O `LlmGateway` precisa ser o único ponto de saída, e `no_egress` e a redação v1 já existem.

### DECISÃO NECESSÁRIA (bloqueia M5)
- **D10: Política de egress.** O texto transcrito pode ir à nuvem por padrão? Quais categorias nunca? Assumido: `cloud-allowed` com redação, e `secret`/`project_context` como `no_egress`.
- **D11: Fallback para nuvem silencioso, perguntar ou bloqueado?** Assumido: nunca silencioso, com padrão "perguntar".
- **D6: Quem faz o fallback.** Reconfirmar, porque o modo degradado depende disso. Assumido: 9router.
- **D4: Papel do LFM3.** Assumido: LLM local de fallback, via `ollamac/*` no 9router.
- **D2: Pessoal ou distribuído.** Define o rigor, a licença do Piper e o auto-update. Assumido: pessoal.

### Tarefas
- [ ] **M5-T01 (L)** Modos de egress por turno: `local-only` (só modelos locais, sem rede externa) e `cloud-allowed` (via 9router, com redação). Aplicados no `LlmGateway`.
- [ ] **M5-T02 (M)** Política de fallback (`perguntar`/`bloquear`/`permitir`), configurável, com padrão "perguntar". Estado `AwaitingEgressApproval` na FSM.
- [ ] **M5-T03 (M)** Indicador visível do modo (cor da orb, ícone ou texto), incluindo o aviso quando o turno caiu para a nuvem ou para o local.
- [ ] **M5-T04 (L)** Redação de egress v2: detectores adicionais, filtro único antes de qualquer chamada de rede, testes com um corpus de segredos sintéticos. Sem log do conteúdo redigido.
- [ ] **M5-T05 (M)** Ollama em LAN e HTTP tratados como egress (rede confiável, mas não local). Se o transporte for HTTP sem TLS, avisar.
- [ ] **M5-T06 (M)** Health check periódico leve do gateway e **modo degradado**: LLM local via `ollamac/lfm3`, ou "só transcrever e mostrar texto".
- [ ] **M5-T07 (M)** Taxonomia de erros completa (`SttFailed`, `SttEmpty`, `LlmTimeout`, `LlmRateLimited`, `GatewayDown`, `ToolDenied`, `ToolFailed`, `TtsFailed`, `AudioDeviceLost`), cada uma com ação definida: retry (só LLM), fallback ou fala de erro.
- [ ] **M5-T08 (M)** Mensagens de erro faladas pré-gravadas para cada erro, independentes do pipeline.
- [ ] **M5-T09 (L)** VAD Silero (`ort`) ou `webrtc-vad`: detecção de fala, corte de silêncio e descarte de capturas sem voz. Substitui a heurística da M1-T06.
- [ ] **M5-T10 (L)** STT por janelas durante a fala (chunks de 2-3 s com overlap) e finalização rápida no release, para reduzir a latência. Só se M1 não atingir o p50 (reavaliar com os dados reais).
- [ ] **M5-T11 (M)** Tratamento de troca de device (headset plugado, mic desconectado) e permissões de microfone do SO, com mensagens claras.
- [ ] **M5-T12 (M)** Retry idempotente: chamadas de LLM têm retry livre, execução de tool nunca. Teste que injeta falha após o efeito.
- [ ] **M5-T13 (S)** Log de auditoria de egress: o que saiu (metadados, tamanho, destino, modo), sem o conteúdo da transcrição por padrão. Visível na UI.
- [ ] **M5-T14 (M)** Teste de invariante: nenhum caminho de código envia áudio à rede (teste de arquitetura, ex. dependências e tipos).
- [ ] **M5-T15 (M)** Reescrever o claim do doc de arquitetura: "áudio nunca sai; texto só sai conforme política".
- [ ] **M5-T16 (M)** Tuning de latência com base nos dados de `tracing`: warmup, tamanho de chunk e escolha do modelo. Atualizar `docs/latencia.md`.

### Definition of Done
- [ ] Em modo `local-only`, o teste de rede mostra zero conexões externas durante um turno completo.
- [ ] Com o gateway derrubado, o app entra em modo degradado em menos de 5 s, avisa visualmente e por voz, e não trava.
- [ ] Nenhuma queda para a nuvem acontece sem o indicador visível. Com a política "perguntar", o fallback aguarda aprovação.
- [ ] O corpus de segredos sintéticos é 100% redigido antes de qualquer chamada de rede (teste automatizado).
- [ ] Todos os erros da taxonomia têm teste e uma ação documentada. A orb nunca fica presa.
- [ ] Silêncio e ruído não geram texto alucinado em 20 capturas de teste (taxa de alucinação 0%).
- [ ] A latência p50 permanece menor que a meta de D7, após o VAD e as janelas.
- [ ] Sem regressão nos testes de segurança do M4.

### Riscos do marco
- **R-M5-1:** fallback silencioso quebra a promessa de privacidade (2.7). Mitigação: T02 e T03.
- **R-M5-2:** retry-storm com fallback em dois níveis (2.8). Mitigação: D6, com um só dono do fallback.
- **R-M5-3:** o texto também é sensível, não só o áudio (2.7). Mitigação: T04 e `no_egress`.
- **R-M5-4:** a redação por regex tem falsos negativos. Mitigação: documentar como best-effort, e `local-only` para trabalho sensível.
- **R-M5-5:** STT por janelas aumenta a complexidade e o risco de regressão na FSM. Mitigação: só entra se o p50 exigir.
- **R-M5-6:** o modelo local (LFM3 1,7B) tem qualidade baixa para PT-BR. Mitigação: validar no spike do M5 antes de depender dele.
- **R-M5-7:** o Silero via `ort` adiciona dependência nativa e risco de empacotamento no Windows.

---

## M6: Polish, empacotamento, extras

**Objetivo:** tornar o app instalável e utilizável sem conhecimento do projeto, e entregar os extras de menor prioridade, cada um independente e cortável.

**Dependências:** M5 concluído para empacotamento. Os extras dependem apenas do M2-M4, conforme o caso.

### DECISÃO NECESSÁRIA (bloqueia M6)
- **D2: Pessoal ou distribuído.**
  - Se houver distribuição: licença GPL do Piper, assinatura do instalador, auto-update e rigor de segurança.
  - Se for pessoal, boa parte das tarefas T03-T06 vira opcional.
- **D13: Descoberta de MCP.** Assumido: só sugere. Auto-ativação seria um risco inaceitável sem sandbox de SO.
- **D17 e D18: Embeddings e sync.** Só viram tarefas se forem confirmados como necessários.
- **D1: SO-alvo.** Se mais de um SO for requisito, o empacotamento se multiplica.

### Tarefas
- [ ] **M6-T01 (L)** **Descoberta de MCP como sugestão:** fontes fechadas (manifestos `.mcp.json` conhecidos, `~/.garagem/tools/` e lista curada de CLIs). Nunca varre o `PATH` inteiro. Registra com `is_active = 0`.
- [ ] **M6-T02 (M)** Fluxo de aprovação de descoberta na UI: mostrar caminho, hash, capabilities e argumentos, com aprovação explícita e pin de hash (reusa M4-T05).
- [ ] **M6-T03 (L)** Downloader de modelos com checksum (SHA-256), retomada, pasta de modelos configurável e verificação de espaço em disco.
- [ ] **M6-T04 (L)** First-run wizard: permissão de microfone, escolha do atalho, download de modelos, teste de áudio, configuração do 9router e chave no keychain.
- [ ] **M6-T05 (L)** Empacotamento do instalador (Tauri bundler) para o SO-alvo, com sidecars (Piper) embutidos, e modelos fora do instalador.
- [ ] **M6-T06 (M)** Auto-update (`tauri-plugin-updater`) com assinatura. Só se D2 indicar distribuição.
- [ ] **M6-T07 (M)** Auditoria de licenças das dependências (`cargo-deny`), com tratamento da GPL do Piper (ou troca do TTS) se houver distribuição.
- [ ] **M6-T08 (M)** Perfis de personalidade nomeados (`profiles`), com troca por voz ou UI.
- [ ] **M6-T09 (M)** Avaliar e, se aprovado, implementar TTS alternativo (Kokoro ou Supertonic) atrás da trait `Tts`.
- [ ] **M6-T10 (L)** Embeddings com `sqlite-vec` e bge-m3 (Ollama), busca híbrida com FTS5. Só com D17 confirmado e dados do M3 mostrando que FTS5 não basta.
- [ ] **M6-T11 (L)** Sandbox de SO para tools (Job Objects/AppContainer no Windows, ou bubblewrap/landlock no Linux). Pré-requisito para qualquer discussão sobre ampliar a autonomia.
- [ ] **M6-T12 (M)** Múltiplos atalhos (ex.: modo `local-only` por um atalho e `cloud-allowed` por outro).
- [ ] **M6-T13 (M)** Polimento da UI: temas, acessibilidade básica e estados vazios e de erro.
- [ ] **M6-T14 (M)** Documentação: README, guia de setup, arquitetura final e runbook de troubleshooting (áudio, atalho, gateway).
- [ ] **M6-T15 (S)** Revisão do orçamento de RAM e CPU em idle e em uso, com os valores finais registrados.

### Definition of Done
- [ ] Uma instalação limpa, no SO-alvo, chega ao primeiro turno falado pelo first-run wizard em menos de 15 minutos (excluindo o tempo de download), sem editar arquivos à mão.
- [ ] Os modelos são baixados e verificados por checksum. Corrompido é rejeitado.
- [ ] A descoberta de MCP nunca ativa uma tool sozinha. Teste automatizado prova que uma tool descoberta fica com `is_active = 0` até a aprovação.
- [ ] `cargo-deny` sem violações. A licença do Piper está tratada conforme D2.
- [ ] Documentação suficiente para um segundo dev reproduzir o setup (revisada por alguém que não escreveu o código).
- [ ] RAM e CPU em idle dentro dos limites registrados no M0 e M1.
- [ ] Os extras T08-T12 são marcados como "entregue" ou "adiado" no backlog, nenhum como "em andamento".

### Riscos do marco
- **R-M6-1:** descoberta de MCP é o maior risco de segurança se mal feita (2.6). Mitigação: só sugestão, hash pin e aprovação.
- **R-M6-2:** licença GPL do Piper em distribuição (2.9). Mitigação: T07 e D2.
- **R-M6-3:** modelos de centenas de MB a GB inviabilizam o instalador (2.9). Mitigação: T03 e T04.
- **R-M6-4:** permissão de microfone e privacidade do SO bloqueiam o first-run (2.9). Mitigação: tratar no wizard.
- **R-M6-5:** escopo dos extras escorrega e atrasa o empacotamento. Mitigação: priorizar T03-T05, e os extras são cortáveis.
- **R-M6-6:** embeddings e sync (D17, D18) são grandes. Mitigação: só entram com a decisão explícita do Diego.

---

## Mapa de dependências

```
M0 --> M1 --> M2 --> M3 --> M4 --> M5 --> M6
                      \_____________^   (M5 exige LlmGateway do M1, no_egress do M3 e a redação v1)
```

- O M3 pode começar em paralelo ao final do M2, depois que o schema estiver estável.
- O M4 não depende do M3 do ponto de vista técnico. Fica depois dele por prioridade, e pode inverter se o caso de uso D20 for uma tool.
- Os extras do M6 (T08-T12) não bloqueiam o empacotamento (T03-T05).

## Decisões pendentes: visão consolidada por marco

| Marco | Decisões que bloqueiam |
|---|---|
| M0 | D1, D3, D7 |
| M1 | D4, D5, D6, D8, D9, D16, D20 |
| M2 | D8, D10 (parcial), D14, D15 |
| M3 | D10, D14, D17, D18, D19 |
| M4 | D9, D12, D13, D20 |
| M5 | D2, D4, D6, D10, D11 |
| M6 | D1, D2, D13, D17, D18 |

## Primeiros passos recomendados

1. Enviar ao Diego a tabela de premissas e obter D1, D3, D7, D5, D6 e D16 antes do M0-T03.
2. Criar os milestones e labels no GitHub, e importar M0-T01 a M0-T12 como issues.
3. Abrir issues `decision-pending` para D1-D20, vinculadas ao marco que cada uma bloqueia.
4. Executar o M0 e recalibrar as estimativas do M1-M6 com a tabela de latências.
