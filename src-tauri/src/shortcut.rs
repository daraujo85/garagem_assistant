// Spike PTT (M0-T03): atalho global com Pressed/Released. Codigo de spike, descartavel exceto as
// medicoes (ver docs/spikes.md). Timestamps, logs e eventos para o frontend.
// M0-T05: Pressed/Released tambem disparam `AudioCapture::start`/`stop` (ver audio/capture.rs; a
// thread dedicada do cpal::Stream, `!Send`, vive la). So para o spike: a FSM de verdade e M1.
// Em M1 esses eventos viram `ShortcutPressed`/`ShortcutReleased` na FSM (ator unico dono do estado).

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use tracing::{error, info, warn};

use crate::audio::capture::AudioCapture;

/// Atalho do push-to-talk. M2: via preferences.
/// Formato aceito por `Shortcut::from_str` (modificadores + tecla, separados por `+`).
///
/// Default trocado de `Ctrl+Shift+Space` para `Ctrl+Alt+Space` apos achado real no M0-T03:
/// `Ctrl+Shift+Space` conflita com outro app (Candy) na maquina de teste (brainstorming 2.1,
/// R-M0-1: "combinacao ja em uso por outro app"). Sobrescrevivel sem recompilar via
/// `GARAGEM_PTT_SHORTCUT`, para testar outras combinacoes se esta tambem colidir.
pub const DEFAULT_PTT_SHORTCUT: &str = "Ctrl+Alt+Space";
pub const PTT_SHORTCUT_ENV: &str = "GARAGEM_PTT_SHORTCUT";

/// Resolve o atalho: env var se definida, senao o default.
pub fn ptt_shortcut() -> String {
    std::env::var(PTT_SHORTCUT_ENV).unwrap_or_else(|_| DEFAULT_PTT_SHORTCUT.to_string())
}

/// Nomes dos eventos Tauri consumidos pelo frontend (`src/App.svelte`).
pub const EVENT_PRESSED: &str = "shortcut-pressed";
pub const EVENT_RELEASED: &str = "shortcut-released";

/// Estado compartilhado do spike: so o instante do primeiro `Pressed` do hold atual.
/// Nao e audio, entao `Mutex` simples basta (o `tauri::State` ja envolve em `Arc`).
/// Em M1 isso sai daqui e passa a viver na FSM (brainstorming 2.1: evitar Mutex espalhado).
#[derive(Default)]
pub struct PttState {
    inner: Mutex<PttInner>,
}

#[derive(Default)]
struct PttInner {
    /// `Some` enquanto o atalho esta pressionado.
    pressed_at: Option<Instant>,
    /// Quantos `Pressed` extras (repeticao de tecla do SO) chegaram durante o hold atual.
    repeats: u32,
}

impl PttState {
    fn lock(&self) -> std::sync::MutexGuard<'_, PttInner> {
        // Poison so ocorre se outra thread deu panic com o lock; para um timestamp, seguir adiante.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Estado do spike de captura (M0-T05): a captura ativa durante o hold, se houver.
/// `AudioCapture` e `Send + Sync` (so canal + JoinHandle; o `cpal::Stream` `!Send` fica na thread
/// de audio), entao pode morar em `tauri::State`. Em M1 vira parte da FSM.
///
/// `Starting` cobre a corrida introduzida ao mover `AudioCapture::start()` para thread propria
/// (ver comentario no handler de `Pressed`): se o usuario soltar o atalho ANTES de `start()`
/// terminar, o `Released` marca `stop_requested=true` em vez de simplesmente nao achar nada para
/// parar — a thread que esta iniciando ve a flag ao terminar e para a captura na hora, em vez de
/// deixar o stream de audio "vazando" (gravando sem ninguem saber).
#[derive(Default)]
enum Capture {
    #[default]
    Idle,
    Starting {
        stop_requested: bool,
    },
    Active(AudioCapture),
}

#[derive(Default)]
pub struct AudioState {
    inner: Mutex<Capture>,
}

impl AudioState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Capture> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Payload de `shortcut-pressed`.
#[derive(Debug, Clone, Serialize)]
struct PressedPayload {
    /// Instante do evento, em ms desde o UNIX epoch (para correlacionar com o console do devtools).
    unix_ms: u128,
}

/// Payload de `shortcut-released`.
#[derive(Debug, Clone, Serialize)]
struct ReleasedPayload {
    unix_ms: u128,
    /// Duracao do hold (Released - primeiro Pressed), em ms com fracao.
    hold_ms: f64,
    /// `Pressed` repetidos pelo SO durante o hold (0 se o SO nao repete).
    repeats: u32,
}

/// Referencia monotonica do spike: `Instant` e opaco no `Debug`, entao logamos ms desde `init_clock`.
static CLOCK_START: OnceLock<Instant> = OnceLock::new();

/// Fixa o t0 dos logs. Chamar uma vez no `setup` (idempotente).
pub fn init_clock() {
    CLOCK_START.get_or_init(Instant::now);
}

/// ms (com fracao, resolucao de `Instant`) desde `init_clock`.
fn t_ms(now: Instant) -> f64 {
    let t0 = *CLOCK_START.get_or_init(Instant::now);
    now.saturating_duration_since(t0).as_secs_f64() * 1000.0
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Plugin com o handler Pressed/Released. O registro do atalho em si e feito por `register`
/// (no `setup`), para uma falha de registro (atalho ja usado por outro app) nao derrubar o app.
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(handle_event)
        .build()
}

/// Registra o atalho resolvido por `ptt_shortcut()`. Chamar no `setup`, depois de
/// `app.manage(PttState::default())`.
pub fn register<R: Runtime>(app: &AppHandle<R>) {
    let shortcut = ptt_shortcut();
    match app.global_shortcut().register(shortcut.as_str()) {
        Ok(()) => info!(shortcut = %shortcut, "atalho global registrado"),
        // Causas tipicas: combinacao ja em uso por outro app (achado real: Ctrl+Shift+Space
        // colidia com o Candy), ou SO/sessao sem suporte (Wayland).
        Err(e) => error!(shortcut = %shortcut, error = %e, "falha ao registrar atalho global"),
    }
}

/// Handler chamado pelo plugin a cada transicao. Roda na thread do plugin: nada de trabalho
/// pesado aqui (so timestamp, log e emit).
fn handle_event<R: Runtime>(app: &AppHandle<R>, shortcut: &Shortcut, event: ShortcutEvent) {
    // Instante capturado o mais cedo possivel, para o jitter nao incluir log/emit.
    let now = Instant::now();

    // So um atalho registrado, mas confere para nao tratar atalhos futuros como PTT.
    match ptt_shortcut().parse::<Shortcut>() {
        Ok(ptt) if &ptt == shortcut => {}
        _ => return,
    }

    let state = app.state::<PttState>();

    match event.state() {
        ShortcutState::Pressed => {
            let mut inner = state.lock();
            if inner.pressed_at.is_some() {
                // Repeticao de tecla do SO durante o hold: nao reinicia o cronometro.
                inner.repeats += 1;
                info!(repeats = inner.repeats, "shortcut Pressed repetido (ignorado)");
                return;
            }
            inner.pressed_at = Some(now);
            inner.repeats = 0;
            drop(inner);

            info!(t_ms = t_ms(now), "shortcut Pressed");
            if let Err(e) = app.emit(EVENT_PRESSED, PressedPayload { unix_ms: unix_ms() }) {
                warn!(error = %e, "falha ao emitir {EVENT_PRESSED}");
            }

            // M0-T05: inicia a captura numa thread separada. ACHADO REAL (teste no Windows,
            // 2026-11-05): `AudioCapture::start()` rodando direto no handler bloqueava o plugin de
            // atalho - o `Released` nunca era processado (UI presa em "escutando"), porque o plugin
            // despacha eventos sequencialmente nessa mesma thread. `start()` abre o device de audio
            // (pode esperar permissao de microfone do SO, driver lento, etc.) e isso e tempo
            // demais para segurar o handler. Mover para thread propria: o handler so dispara e
            // retorna, o resultado chega via AudioState quando `start()` terminar.
            *app.state::<AudioState>().lock() = Capture::Starting { stop_requested: false };
            let app_handle = app.clone();
            let t = Instant::now();
            std::thread::spawn(move || {
                let result = AudioCapture::start();
                let audio_state = app_handle.state::<AudioState>();
                let mut guard = audio_state.lock();
                // Enquanto `start()` rodava, o Released pode ter chegado e marcado `stop_requested`
                // (usuario soltou rapido demais). Nesse caso a captura acabou de nascer mas ja deve
                // morrer, senao o audio ficaria gravando escondido sem ninguem para para-lo.
                let stop_requested = matches!(*guard, Capture::Starting { stop_requested: true });
                match result {
                    Ok(capture) => {
                        info!(
                            start_ms = t.elapsed().as_secs_f64() * 1000.0,
                            stop_requested,
                            "captura de audio iniciada"
                        );
                        if stop_requested {
                            *guard = Capture::Idle;
                            drop(guard);
                            if let Err(e) = capture.stop() {
                                error!(error = %e, "falha ao finalizar captura (stop imediato pos-start)");
                            }
                        } else {
                            *guard = Capture::Active(capture);
                        }
                    }
                    Err(e) => {
                        *guard = Capture::Idle;
                        error!(error = %e, "falha ao iniciar captura de audio");
                    }
                }
            });
        }
        ShortcutState::Released => {
            let mut inner = state.lock();
            let pressed_at = inner.pressed_at.take();
            let repeats = std::mem::take(&mut inner.repeats);
            drop(inner);

            let Some(pressed_at) = pressed_at else {
                // Released sem Pressed: evento perdido ou duplicado. Relevante pro go/no-go.
                warn!(t_ms = t_ms(now), "shortcut Released sem Pressed correspondente");
                return;
            };

            let hold_ms = now.duration_since(pressed_at).as_secs_f64() * 1000.0;
            info!(t_ms = t_ms(now), hold_ms, repeats, "shortcut Released");
            let payload = ReleasedPayload { unix_ms: unix_ms(), hold_ms, repeats };
            if let Err(e) = app.emit(EVENT_RELEASED, payload) {
                warn!(error = %e, "falha ao emitir {EVENT_RELEASED}");
            }

            // M0-T05: para a captura numa thread separada (mesmo motivo do Pressed: `stop()` e
            // bloqueante - junta o consumidor e escreve o WAV em disco - e nao pode travar o
            // handler do plugin de atalho).
            let audio_state = app.state::<AudioState>();
            let mut guard = audio_state.lock();
            match std::mem::take(&mut *guard) {
                Capture::Active(capture) => {
                    drop(guard);
                    std::thread::spawn(move || {
                        let t = Instant::now();
                        match capture.stop() {
                            Ok(path) => info!(
                                wav = %path.display(),
                                stop_ms = t.elapsed().as_secs_f64() * 1000.0,
                                hold_ms,
                                "WAV gerado"
                            ),
                            Err(e) => error!(error = %e, "falha ao finalizar captura de audio"),
                        }
                    });
                }
                Capture::Starting { .. } => {
                    // `start()` ainda rodando: marca para a thread do Pressed parar assim que
                    // terminar, em vez de deixar a captura vazando.
                    *guard = Capture::Starting { stop_requested: true };
                    warn!("Released durante o Starting da captura; stop agendado para quando iniciar");
                }
                Capture::Idle => warn!("Released sem captura ativa (start falhou ou ainda nao chamado?)"),
            }
        }
    }
}
