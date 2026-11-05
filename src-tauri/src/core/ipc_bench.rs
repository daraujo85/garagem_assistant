// Spike de IPC (M0-T10): `emit` (eventos Tauri) x `tauri::ipc::Channel` (streaming tipado) para
// dados de alta frequencia Rust -> Svelte. Benchmark SINTETICO e isolado: nao toca em audio/capture.rs.
// Contexto: brainstorming 2.2 ("nao mande evento Tauri por frame de audio; `emit` serializa JSON via
// IPC e a 60+ Hz satura e gera jitter") e plano M0-T10. O resultado decide o transporte dos niveis
// de audio (RMS/bandas) que o M1 manda ao orb.
//
// ATENCAO (API a confirmar no `cargo build` do Windows; este arquivo foi escrito sem Rust no WSL2):
//  - `tauri::ipc::Channel<T>`: `T` precisa de `Serialize` (no Tauri 2.x o bound e `IpcResponse`, que
//    qualquer `Serialize` satisfaz). `Channel::send(&self, T) -> tauri::Result<()>`; o `Channel` e
//    `Clone + Send + 'static`, entao pode ser movido para uma `std::thread`. Se o bound mudar na
//    versao instalada, ajustar `BenchPayload`.
//  - No JS o canal e passado como argumento normal do `invoke` (`{ channel }`), nome do parametro
//    Rust (`channel`) igual a chave. O parametro `duration_secs` vira `durationSecs` no JS (camelCase
//    padrao do Tauri 2).
//  - `AppHandle::emit` exige `use tauri::Emitter` (trait; no Tauri 2 saiu de `Manager`). `emit`
//    faz broadcast para TODAS as janelas (inclui a "orb"); o custo de serializar JSON e o mesmo.
//  - Parametros/retorno dos comandos: `Result<(), String>` e aceito pelo `#[tauri::command]`.
//
// Medicao no lado Rust: cada transporte conta quantos envios de fato sairam vs os esperados
// (`hz * duration_secs`) e o maior atraso de agendamento (quanto a thread acordou depois do instante
// previsto), para separar atraso do PROPRIO Rust do jitter do IPC/WebView2. Ao fim, o resumo vai ao
// frontend no evento `bench-done` (1 mensagem, nao interfere na medicao) e para o log (tracing).

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

/// Evento por amostra (transporte `emit`). Consumido em `src/App.svelte`.
pub const EVENT_LEVEL: &str = "bench-level";
/// Evento de fim de execucao, com o resumo medido no Rust (`BenchReport`). Usado nos DOIS
/// transportes: e 1 so mensagem por execucao, entao nao distorce o jitter medido.
pub const EVENT_DONE: &str = "bench-done";

/// Folga de spin no fim da espera. `thread::sleep` no Windows tem granularidade ~15,6 ms (timer
/// padrao do SO), o que por si so geraria jitter de ~1 frame a 60 Hz e mascararia o IPC. Dormimos
/// ate `deadline - SPIN_MARGIN` e giramos (`yield_now`) o resto. Custa CPU so durante o benchmark.
const SPIN_MARGIN: Duration = Duration::from_millis(2);

/// Limites de sanidade dos parametros vindos do frontend.
const MAX_HZ: u32 = 240;
const MAX_DURATION_SECS: u32 = 60;

/// Impede dois benchmarks simultaneos (um contaminaria a medicao do outro).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Payload sintetico, pequeno como um frame de nivel de audio. Mesmo tipo nos dois transportes.
#[derive(Debug, Clone, Serialize)]
pub struct BenchPayload {
    /// Numero de sequencia (0..). Lacunas no frontend = perda/descarte.
    pub seq: u64,
    /// "Nivel RMS" falso (onda senoidal 0..1), so para o payload ter conteudo realista.
    pub value: f32,
    /// Instante do envio, em ms desde o UNIX epoch (com fracao). O frontend compara com
    /// `performance.timeOrigin + performance.now()` para estimar a latencia de entrega.
    pub sent_at_ms: f64,
}

/// Resumo medido no Rust ao fim da execucao.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    /// "emit" ou "channel".
    pub transport: &'static str,
    pub hz: u32,
    pub duration_secs: u32,
    /// `hz * duration_secs`.
    pub expected: u64,
    /// Envios que retornaram `Ok` (nao garante que o frontend recebeu).
    pub sent: u64,
    /// Envios que retornaram `Err`.
    pub send_errors: u64,
    /// Duracao real da execucao no Rust, em ms (esperado ~ `duration_secs * 1000`).
    pub elapsed_ms: f64,
    /// Maior atraso do envio em relacao ao instante agendado (ms). Alto = o proprio Rust atrasou.
    pub max_lag_ms: f64,
    /// Maior duracao de uma chamada `emit`/`send` (ms): custo de serializar + enfileirar no IPC.
    pub max_send_ms: f64,
    /// Media da duracao de `emit`/`send` (ms).
    pub avg_send_ms: f64,
}

/// Libera o flag `RUNNING` mesmo se a thread der panic.
struct RunGuard;
impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

fn validate(hz: u32, duration_secs: u32) -> Result<(), String> {
    if hz == 0 || hz > MAX_HZ {
        return Err(format!("hz deve estar em 1..={MAX_HZ} (recebido {hz})"));
    }
    if duration_secs == 0 || duration_secs > MAX_DURATION_SECS {
        return Err(format!(
            "duration_secs deve estar em 1..={MAX_DURATION_SECS} (recebido {duration_secs})"
        ));
    }
    Ok(())
}

fn try_acquire() -> Result<RunGuard, String> {
    RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map(|_| RunGuard)
        .map_err(|_| "ja existe um benchmark de IPC em execucao".to_string())
}

fn unix_ms_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Espera ate `deadline`: dorme a maior parte e gira o resto (ver `SPIN_MARGIN`).
fn wait_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now + SPIN_MARGIN {
        thread::sleep(deadline - now - SPIN_MARGIN);
    }
    while Instant::now() < deadline {
        thread::yield_now();
    }
}

/// Laco comum aos dois transportes: agenda `hz * duration_secs` envios em instantes FIXOS
/// (`start + n * periodo`, sem acumular deriva) e chama `send` para cada payload. Devolve o resumo.
/// `send` devolve `true` se o envio deu certo.
fn run_bench(
    transport: &'static str,
    hz: u32,
    duration_secs: u32,
    mut send: impl FnMut(BenchPayload) -> bool,
) -> BenchReport {
    let expected = u64::from(hz) * u64::from(duration_secs);
    let period = Duration::from_secs_f64(1.0 / f64::from(hz));

    let mut sent = 0u64;
    let mut send_errors = 0u64;
    let mut max_lag = Duration::ZERO;
    let mut max_send = Duration::ZERO;
    let mut total_send = Duration::ZERO;

    let start = Instant::now();
    for seq in 0..expected {
        let deadline = start + period.mul_f64(seq as f64);
        wait_until(deadline);

        let lag = Instant::now().saturating_duration_since(deadline);
        max_lag = max_lag.max(lag);

        let t = seq as f64 / f64::from(hz);
        let payload = BenchPayload {
            seq,
            // Senoide 0..1 a 1 Hz.
            value: (0.5 + 0.5 * (t * std::f64::consts::TAU).sin()) as f32,
            sent_at_ms: unix_ms_now(),
        };

        let t0 = Instant::now();
        let ok = send(payload);
        let took = t0.elapsed();
        max_send = max_send.max(took);
        total_send += took;

        if ok {
            sent += 1;
        } else {
            send_errors += 1;
        }
    }

    let report = BenchReport {
        transport,
        hz,
        duration_secs,
        expected,
        sent,
        send_errors,
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        max_lag_ms: max_lag.as_secs_f64() * 1000.0,
        max_send_ms: max_send.as_secs_f64() * 1000.0,
        avg_send_ms: if expected > 0 {
            total_send.as_secs_f64() * 1000.0 / expected as f64
        } else {
            0.0
        },
    };
    info!(
        transport,
        hz,
        expected,
        sent,
        send_errors,
        elapsed_ms = report.elapsed_ms,
        max_lag_ms = report.max_lag_ms,
        avg_send_ms = report.avg_send_ms,
        max_send_ms = report.max_send_ms,
        "bench IPC concluido"
    );
    report
}

/// Transporte A: `app.emit("bench-level", payload)` a `hz` Hz por `duration_secs` s.
/// Retorna assim que a thread de envio e criada; o resumo chega depois em `bench-done`.
#[tauri::command]
pub fn start_emit_bench(app: AppHandle, hz: u32, duration_secs: u32) -> Result<(), String> {
    validate(hz, duration_secs)?;
    let guard = try_acquire()?;

    thread::Builder::new()
        .name("ipc-bench-emit".into())
        .spawn(move || {
            let _guard = guard;
            let report = run_bench("emit", hz, duration_secs, |payload| {
                match app.emit(EVENT_LEVEL, payload) {
                    Ok(()) => true,
                    Err(e) => {
                        warn!("emit falhou: {e}");
                        false
                    }
                }
            });
            if let Err(e) = app.emit(EVENT_DONE, report) {
                warn!("emit do resumo falhou: {e}");
            }
        })
        .map_err(|e| format!("falha ao criar thread do bench: {e}"))?;
    Ok(())
}

/// Transporte B: `channel.send(payload)` a `hz` Hz por `duration_secs` s. Mesmo laco e mesmo
/// payload do `emit`. O `app` so serve para emitir o resumo final (`bench-done`).
#[tauri::command]
pub fn start_channel_bench(
    app: AppHandle,
    channel: Channel<BenchPayload>,
    hz: u32,
    duration_secs: u32,
) -> Result<(), String> {
    validate(hz, duration_secs)?;
    let guard = try_acquire()?;

    thread::Builder::new()
        .name("ipc-bench-channel".into())
        .spawn(move || {
            let _guard = guard;
            let report = run_bench("channel", hz, duration_secs, |payload| {
                match channel.send(payload) {
                    Ok(()) => true,
                    Err(e) => {
                        warn!("channel.send falhou: {e}");
                        false
                    }
                }
            });
            if let Err(e) = app.emit(EVENT_DONE, report) {
                warn!("emit do resumo falhou: {e}");
            }
        })
        .map_err(|e| format!("falha ao criar thread do bench: {e}"))?;
    Ok(())
}
