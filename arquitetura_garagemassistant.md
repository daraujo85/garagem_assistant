# GaragemAssistant: Arquitetura e Setup de Projeto

O **GaragemAssistant** é um assistente de desktop focado em produtividade para desenvolvedores. Ele roda localmente, possui memória persistente, suporte a descoberta dinâmica de ferramentas (MCPs/CLIs) e uma interface fluida sem travamentos. O acionamento é feito estritamente via atalho de teclado (Global Shortcuts), sem uso de wake-words, garantindo privacidade total e baixo consumo de CPU em repouso.

## 1. Stack Tecnológico

*   **Frontend (Apresentação Visual):** Tauri + Vue.js (ou Svelte) + WebGL/Three.js (para animação da bolinha de status via GPU).
*   **Backend (Core Orquestrador):** Rust + Tokio (assíncrono).
*   **Áudio (Baixa Latência):** `cpal` (captura mic), `ringbuf` (buffer lock-free), `rodio` (playback TTS).
*   **Memória e Estado:** SQLite via `sqlx`.
*   **IA e Pipeline:** 
    *   **STT (Transcrição):** Whisper.cpp / LFM3 local.
    *   **Roteamento e Intenção:** Jev + Nine-router (com fallback para Ollama/OpenAI).
    *   **TTS (Voz):** Piper (geração super rápida de PCM).
    *   **Motor de Prompt (Personalidade):** `minijinja` (Template engine do Rust).

---

## 2. Estrutura de Diretórios do Projeto

Abaixo está o esqueleto de como o monorepo Tauri deve ser organizado para manter o core limpo.

```text
garagem-assistant/
├── src/                      # Frontend (Vue/Svelte/React)
│   ├── components/
│   │   ├── OrbVisualizer.vue # Componente WebGL que renderiza a bolinha
│   │   ├── SettingsPanel.vue # Sliders de personalidade e toggles de MCPs
│   ├── assets/
│   └── main.js
├── src-tauri/                # Backend (Rust)
│   ├── src/
│   │   ├── core/             # O "Cérebro"
│   │   │   ├── mcp_discovery.rs # Varredura de executáveis e manifestos
│   │   │   ├── prompt_engine.rs # Minijinja templates para personalidade
│   │   │   └── dispatcher.rs    # Roteador de intenção (Jev)
│   │   ├── audio/            
│   │   │   ├── capture.rs       # Setup do cpal e ringbuf
│   │   │   └── tts_piper.rs     # Integração e playback da resposta
│   │   ├── db/
│   │   │   ├── models.rs        # Structs das tabelas
│   │   │   └── memory_store.rs  # Queries do sqlx (RAG local)
│   │   └── main.rs           # Entrypoint do Tauri, Setup de Shortcuts
│   ├── migrations/           # Arquivos SQL de migração do sqlx
│   ├── Cargo.toml
│   └── tauri.conf.json
└── mcps.json                 # Arquivo de configuração de skills/ferramentas locais
```

---

## 3. Esquema do Banco de Dados (SQLite)

O SQLite atuará como o disco rígido do agente. Use o `sqlx` para gerenciar as migrações.

**Tabela: `preferences` (Personalidade e Configuração)**
```sql
CREATE TABLE preferences (
    id TEXT PRIMARY KEY,
    key TEXT UNIQUE NOT NULL,
    value TEXT NOT NULL -- Ex: humor (0-100), explicit_allowed (true/false), agent_name
);
```

**Tabela: `memories` (Contexto Retido)**
```sql
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL, -- 'secret', 'project_context', 'fact'
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    last_accessed DATETIME DEFAULT CURRENT_TIMESTAMP,
    access_count INTEGER DEFAULT 0
);
```

**Tabela: `mcp_registry` (Ferramentas e Skills)**
```sql
CREATE TABLE mcp_registry (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    command_path TEXT NOT NULL,
    transport TEXT NOT NULL, -- 'stdio', 'http'
    is_active BOOLEAN DEFAULT 1
);
```

---

## 4. Dependências Principais (Cargo.toml)

No diretório `src-tauri`, o seu `Cargo.toml` precisará das seguintes crates para viabilizar a arquitetura:

```toml
[dependencies]
# Framework base
tauri = { version = "2.0.0", features = ["global-shortcut"] }
tokio = { version = "1.36", features = ["full"] }

# Banco de dados e Estado
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Áudio de baixa latência
cpal = "0.15"
ringbuf = "0.3"
rodio = "0.17" # Para tocar o áudio do Piper

# Motor de Personalidade (Templates)
minijinja = "1.0"

# Concorrência e Canais
crossbeam-channel = "0.5"

# Execução de subprocessos (MCP e Skills)
async-process = "2.1"
```

---

## 5. Dinâmica do Motor de Personalidade

Em vez de usar strings estáticas para o System Prompt, o arquivo `prompt_engine.rs` usará o `minijinja`. Quando o usuário aciona o microfone, o Rust faz um select em `preferences` e passa as variáveis para o template:

```text
// Template base do Sistema (Exemplo Minijinja)
Você é o {{ agent_name }}, um assistente de engenharia de software rodando localmente.

Regras de Personalidade:
{% if humor > 80 %}
Você é extremamente sarcástico, direto ao ponto e não tem paciência para enrolação.
{% elif humor > 40 %}
Você é amigável e colaborativo.
{% else %}
Você é estritamente profissional, corporativo e focado apenas nos fatos técnicos.
{% endif %}

{% if explicit_allowed %}
O uso de palavrões e jargões informais está LIBERADO se o contexto da conversa pedir (ex: frustração com bugs).
{% else %}
Mantenha uma linguagem completamente limpa e polida, sem qualquer profanidade.
{% endif %}

Contexto Recuperado da sua Memória Local (Use se necessário):
{{ injected_memories }}
```

## 6. Ciclo de Interação (Push-to-Talk)

1.  **Standby:** Aplicação consome quase 0% de CPU. A bolinha na UI (WebGL) pulsa suavemente na cor cinza.
2.  **Ativação:** Usuário segura `Ctrl + Shift + Espaço` (ou outro atalho configurado).
3.  **Escuta Lock-Free:** O Rust dispara um evento para a UI (Bolinha fica colorida e reage ao volume). O `cpal` joga o áudio no `ringbuf`.
4.  **Processamento:** Ao soltar a tecla, o áudio vai pro Whisper/LFM3 -> Jev -> Roteador (LLM + MCP + Memória SQLite). A UI muda para estado "Pensando" (giro orbital).
5.  **Resposta:** Piper gera o áudio, Rust toca via `rodio` e emite os picos de frequência para a UI (Bolinha "fala").