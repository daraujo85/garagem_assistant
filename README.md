# GaragemAssistant

Assistente de desktop local focado em produtividade para desenvolvedores. Roda localmente, com memória persistente, descoberta dinâmica de ferramentas (MCPs/CLIs) e interface fluida sem travamentos. Acionamento estritamente por atalho de teclado (push-to-talk), sem wake-word — privacidade alta e baixo consumo de CPU em repouso.

Plataforma-alvo inicial: **Windows**. Suporte a Linux/macOS fica para depois (ver `docs/plano.md`).

## Status

Projeto em fase de planejamento. Nenhum código de aplicação ainda — ver [`docs/plano.md`](docs/plano.md), marco **M0 (spikes de risco)**.

## Stack

- **Frontend:** Tauri + Svelte + WebGL (orb de status)
- **Backend:** Rust + Tokio
- **Áudio:** `cpal` (captura), `ringbuf` (buffer lock-free), `rodio` (playback)
- **STT:** Whisper.cpp (PT-BR)
- **TTS:** Piper
- **Memória/Estado:** SQLite
- **Personalidade:** `minijinja` (templates de prompt)

Detalhes completos em [`arquitetura_garagemassistant.md`](arquitetura_garagemassistant.md).

## Documentação

- [`arquitetura_garagemassistant.md`](arquitetura_garagemassistant.md) — arquitetura original proposta (stack, estrutura de diretórios, schema do banco, ciclo de interação)
- [`docs/brainstorming.md`](docs/brainstorming.md) — brainstorming crítico: riscos técnicos, decisões de stack questionadas, alternativas e perguntas em aberto
- [`docs/plano.md`](docs/plano.md) — plano de projeto dividido em marcos (M0–M6) e tarefas, com critérios de pronto e riscos por marco

## Decisões já confirmadas

- SO-alvo inicial: **Windows**
- Idioma: **PT-BR**
- Hardware: GPU NVIDIA disponível (CUDA) para aceleração de STT
- Frontend: **Svelte**

Demais decisões em aberto (papel do Jev, política de egress, sqlx vs rusqlite etc.) estão documentadas em `docs/plano.md`, seção "Premissas provisórias".
