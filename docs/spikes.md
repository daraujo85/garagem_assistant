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
