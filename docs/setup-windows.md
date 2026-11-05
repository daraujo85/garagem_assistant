# Setup no Windows

> **AVISO: scaffolding NAO validado.** O scaffolding do M0-T01 foi escrito a mao em WSL2 (sem Rust/cargo nem deps de Node instaladas), sem nunca ter sido compilado. Antes de seguir para o resto do M0, valide tudo no Windows seguindo este doc. Erros de versao de crate, de config do Tauri ou de sintaxe sao esperados e devem ser corrigidos primeiro.

O SO-alvo e Windows 10/11 x64. Build e execucao acontecem no host Windows. WSL2 serve apenas para edicao de codigo.

## Pre-requisitos

1. **Visual Studio Build Tools** (2019 ou 2022) com a carga de trabalho "Desenvolvimento para desktop com C++" (MSVC e Windows SDK).
2. **WebView2 Runtime**. Ja vem no Windows 11 e em Windows 10 atualizado. Se faltar, instale o Evergreen Runtime da Microsoft.
3. **Rust** via [rustup](https://rustup.rs), toolchain `stable-msvc` (padrao no Windows). Minimo: 1.77.
4. **Node.js** LTS (inclui npm).

Guia oficial: https://tauri.app/start/prerequisites/

## Rodar

No PowerShell, na raiz do repo:

```powershell
npm install
npm run tauri dev
```

A primeira compilacao Rust baixa e compila todas as crates (varios minutos; `rusqlite` com `bundled` compila o SQLite em C).

Resultado esperado: janela "GaragemAssistant" com um circulo cinza e o texto "Estado: Idle" (prova do IPC com o comando `get_state`).

## Checklist de validacao (pendente)

- [ ] `npm install` conclui sem erros.
- [ ] `cargo check` em `src-tauri` passa (ver pontos de atencao abaixo).
- [ ] `npm run tauri dev` abre a janela com "Estado: Idle".
- [ ] Gerar icones definitivos: `npm run tauri icon caminho/para/logo.png` (os atuais sao placeholders gerados por script).
- [ ] `cargo fmt --check` e `cargo clippy -- -D warnings`.

## Pontos de atencao conhecidos

- **Versoes de crates** foram escolhidas de memoria (rusqlite 0.32 + rusqlite_migration 1.3, ringbuf 0.4, rodio 0.20, minijinja 2, thiserror 2). `rusqlite_migration` deve casar com a versao do `rusqlite`; ajuste se o resolvedor reclamar.
- **Modulo `brain`**: a pasta `src-tauri/src/core/` e declarada como `mod brain` via `#[path]` para nao sombrear o crate `core` da std.
- **Capabilities**: `src-tauri/capabilities/default.json` usa `global-shortcut:default`. Confirme o nome da permissao no schema gerado em `src-tauri/gen/schemas`.
- **Icones**: `src-tauri/icons/` contem PNGs/ICO minimos gerados por script, so para o build nao falhar.
- Sem `Cargo.lock` e sem `package-lock.json`: serao gerados no primeiro build e devem ser commitados.
