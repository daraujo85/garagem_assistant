// Spike de playback (M0-T08). Codigo de spike, descartavel exceto as medicoes (ver docs/spikes.md,
// secao "Spike Playback"). Escopo: tocar o WAV gerado pelo TTS (M0-T07) por dois backends
// selecionaveis e medir a latencia de stop (base do barge-in do M1, brainstorming 2.4: < 200 ms).
// SEM FSM, SEM streaming, SEM barge-in integrado (M1-T10/T11).
//
// ATENCAO (API do rodio 0.20): este arquivo foi escrito em WSL2 SEM cargo (nao compilado). A versao
// exata do rodio e da API (`OutputStream::try_default()` + `Sink::try_new(&handle)` + `Decoder::new`)
// PRECISA ser confirmada no primeiro `cargo build` do Windows: ate onde se sabe esse e o padrao do
// 0.17-0.20, e a refatoracao para `OutputStreamBuilder::open_default_stream()` / `Sink::connect_new`
// veio depois (0.21+), mas isso nao foi verificado aqui. Se divergir, ajustar `rodio_control`/
// `rodio_stop`. O caminho cpal reusa exatamente os padroes de capture.rs (mesmo crate,
// mesma versao), entao deve compilar de primeira.
//
// Perguntas do plano M0-T08:
//   (A) `rodio` (out) e `cpal` (in) coexistem no mesmo device?  -> comando `spike_conflict_test`
//       + teste manual (segurar PTT enquanto toca). Ver docs/spikes.md.
//   (B) Latencia real de parar o playback (barge-in)            -> `stop()` retorna `stop_latency_ms`.
//
// Arquitetura (mesma regra do capture.rs, brainstorming 2.1):
//
//   [handle] --(crossbeam-channel: Stop/Shutdown)--> [thread de audio, dona do stream]
//                                                          |
//                                        Rodio: OutputStream + Sink (device proprio do rodio)
//                                        Cpal: cpal::Stream de saida consumindo ringbuf SPSC
//                                              preenchido com o WAV decodificado + resample
//
// Aproximacoes das medicoes (documentadas tambem em docs/spikes.md):
// - `start_ms`: tempo do `play_wav` ate o stream estar iniciado (`stream.play()` retornou / Sink
//   populado). NAO e o primeiro sample audivel: falta ~1 periodo do device (5-20 ms tipico).
// - `first_sample_ms` (so cpal): momento da PRIMEIRA invocacao do callback de saida, que e quando
//   o device comecou a puxar samples — melhor aproximacao do primeiro som observavel de dentro do
//   Rust. No rodio nao ha como observar isso sem envolver a source; fica `None`.
// - `stop_latency_ms`:
//   * Cpal: tempo entre o pedido de stop e o `cpal::Stream` ser DROPADO (o WASAPI encerra o
//     stream). O flag atomico tambem e setado antes, entao o callback ja pararia de alimentar o
//     device no proximo periodo (~10 ms) mesmo sem o drop. O buffer interno do WASAPI (modo
//     compartilhado, tipicamente 10-20 ms) pode ainda soar por um instante; isso NAO e observavel
//     de dentro do Rust.
//   * Rodio: tempo entre o pedido de stop e o `Sink` reportar vazio (`sink.empty()`, poll de 1 ms).
//     SUBESTIMA o stop acustico real: o rodio nao expoe o preenchimento do buffer do device, entao
//     "sink vazio" acontece antes do ultimo sample sair da placa. Comparar com o cpal e com o
//     ouvido (gravar com o celular se preciso).
//
// Regra de canais (brainstorming 2.1): crossbeam-channel entre handle e thread de audio; nada de
// tokio::sync aqui. Comandos Tauri async usam `spawn_blocking` (o `stop()` e bloqueante).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use hound::{SampleFormat as HoundFormat, WavReader};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapRb};
use rubato::{FftFixedIn, Resampler};
use serde::Serialize;
use thiserror::Error;
use tracing::{error, info, warn};

use crate::audio::capture::AudioCapture;

/// Intervalo de polling da thread de controle quando nada acontece (atualiza `is_playing`).
const CONTROL_POLL: Duration = Duration::from_millis(50);

/// Intervalo de polling do `sink.empty()` no stop do rodio.
const RODIO_STOP_POLL: Duration = Duration::from_millis(1);

/// Folga no fim natural do audio antes de derrubar o stream (buffer do device ainda tem ~10-20 ms).
const TAIL_GRACE: Duration = Duration::from_millis(50);

/// Teto de espera pelo sink esvaziar no stop do rodio (evita travar o comando pra sempre).
const RODIO_STOP_TIMEOUT: Duration = Duration::from_millis(1000);

/// Frames de entrada por chunk do resampler (mesma constante de capture.rs: ~21 ms a 48 kHz).
const RESAMPLE_CHUNK_FRAMES: usize = 1024;

/// Buffer de conversao no callback de saida (mono, alocado uma vez na criacao do closure).
const CALLBACK_SCRATCH: usize = 4096;

#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("arquivo WAV nao encontrado: '{path}'")]
    NotFound { path: String },
    #[error("falha ao ler o WAV '{path}': {source}")]
    Wav { path: String, source: hound::Error },
    #[error("WAV '{path}' nao tem samples")]
    EmptyWav { path: String },
    #[error("WAV '{path}': {bits} bits por sample nao suportado (use 8/16/24/32 int ou 32 float)")]
    UnsupportedWavBits { path: String, bits: u16 },
    #[error("nenhum dispositivo de saida de audio encontrado")]
    NoOutputDevice,
    #[error("falha ao consultar a configuracao do dispositivo de saida: {0}")]
    DeviceConfig(String),
    #[error("formato de stream de saida nao suportado: {0}")]
    UnsupportedFormat(String),
    #[error("falha ao abrir o stream de saida: {0}")]
    BuildStream(String),
    #[error("falha ao iniciar o stream de saida: {0}")]
    PlayStream(String),
    #[error("falha no OutputStream do rodio: {0}")]
    OutputStream(String),
    #[error("falha no Sink do rodio: {0}")]
    Sink(String),
    #[error("falha ao decodificar o WAV com o rodio '{path}': {reason}")]
    Decode { path: String, reason: String },
    #[error("falha no resampler: {0}")]
    Resampler(String),
    #[error("backend invalido: '{0}' (use 'rodio' ou 'cpal')")]
    InvalidBackend(String),
    #[error("falha ao criar a thread de audio: {0}")]
    ThreadSpawn(std::io::Error),
    #[error("a thread de audio encerrou (canal fechado)")]
    ChannelClosed,
    #[error("nenhum playback em andamento")]
    NotPlaying,
}

/// Backends do spike (plano M0-T08: rodio E cpal, selecionaveis, para comparar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PlaybackBackend {
    /// `rodio`: abre o proprio output stream (brainstorming 2.1: pode brigar com o cpal in em
    /// WASAPI exclusivo — o spike existe exatamente para testar isso).
    Rodio,
    /// `cpal` puro: stream de saida nosso, ringbuf SPSC, controle direto do stop (alternativa
    /// recomendada pelo brainstorming 3: "usar so cpal (in e out), mais controle de latencia").
    Cpal,
}

impl PlaybackBackend {
    /// Parse do nome vindo do frontend ("rodio" | "cpal").
    pub fn parse(s: &str) -> Result<Self, PlaybackError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rodio" => Ok(Self::Rodio),
            "cpal" => Ok(Self::Cpal),
            other => Err(PlaybackError::InvalidBackend(other.to_string())),
        }
    }
}

/// Metricas do inicio do playback (retorno do comando `play_wav_rust`).
#[derive(Debug, Clone, Serialize)]
pub struct PlaybackInfo {
    pub backend: PlaybackBackend,
    /// Duracao do WAV lida do header (hound).
    pub wav_secs: f32,
    /// APROXIMACAO: `play_wav` ate o stream iniciar (ver nota no topo do arquivo). Nao inclui o
    /// periodo do device.
    pub start_ms: u64,
    /// So cpal: momento da primeira invocacao do callback de saida (ver nota no topo). `None` no
    /// rodio (nao observavel) e enquanto o primeiro callback nao rodou.
    pub first_sample_ms: Option<u64>,
}

/// Metricas do stop (retorno do comando `stop_playback`; base da tabela go/no-go do spike).
#[derive(Debug, Clone, Serialize)]
pub struct StopReport {
    pub backend: PlaybackBackend,
    /// Pergunta (B) do spike: ms do pedido de stop ate o audio "parar de fato" (aproximacoes por
    /// backend documentadas no topo do arquivo e em docs/spikes.md).
    pub stop_latency_ms: u64,
    /// Rodio: o sink chegou a esvaziar dentro do teto de 1 s. Cpal: sempre true (o stream e
    /// dropado imediatamente).
    pub drained: bool,
    /// Cpal: samples que estavam no ringbuf e nao chegaram a tocar quando o stop cortou (o
    /// "resto" descartado pelo barge-in). Rodio: None (nao observavel).
    pub samples_undrained: Option<u64>,
}

/// Comandos da thread de audio (mesmo padrao do capture.rs). Cada `Stop` leva canal one-shot.
enum Command {
    Stop(Sender<Result<StopReport, PlaybackError>>),
    Shutdown,
}

/// Handle do playback. `Send` (so canal + JoinHandle + atomics); o `cpal::Stream` `!Send` e o
/// `OutputStream` do rodio nunca saem da thread de audio. `Drop` envia `Shutdown` e junta a thread
/// (mesma semantica do `AudioCapture`).
pub struct PlaybackHandle {
    backend: PlaybackBackend,
    tx: Sender<Command>,
    handle: Option<JoinHandle<()>>,
    /// Espelho do estado "tocando" mantido pela thread de controle (lido por `is_playing`).
    playing: Arc<AtomicBool>,
    /// Metricas atualizadas pela thread (ex.: `first_sample_ms` chega async no cpal).
    info: Arc<Mutex<Option<PlaybackInfo>>>,
}

impl PlaybackHandle {
    /// Toca o WAV em `path` no backend escolhido e devolve o handle. Bloqueia ate o stream estar
    /// iniciado (ou devolve o erro de abertura do device), igual ao `AudioCapture::start`.
    pub fn play_wav(path: &Path, backend: PlaybackBackend) -> Result<Self, PlaybackError> {
        let path = path.to_path_buf();
        if !path.is_file() {
            return Err(PlaybackError::NotFound { path: path.display().to_string() });
        }

        // Duracao do header: valida o WAV cedo e alimenta a metrica (ambos os backends).
        let wav_secs = wav_duration_secs(&path)?;

        let (tx, rx) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let playing = Arc::new(AtomicBool::new(false));
        let info: Arc<Mutex<Option<PlaybackInfo>>> = Arc::new(Mutex::new(None));

        // No cpal o WAV e decodificado ANTES de spawnar a thread (erro de formato volta do
        // `play_wav`, nao async). O rodio decodifica por streaming, entao so leva o path.
        let decoded = match backend {
            PlaybackBackend::Cpal => {
                let Some((samples, rate)) = decode_wav_mono(&path)? else {
                    return Err(PlaybackError::EmptyWav { path: path.display().to_string() });
                };
                Some((samples, rate))
            }
            PlaybackBackend::Rodio => None,
        };

        let thread_playing = Arc::clone(&playing);
        let thread_info = Arc::clone(&info);
        let spawned = thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || match backend {
                PlaybackBackend::Rodio => {
                    rodio_control(&path, wav_secs, ready_tx, rx, thread_playing, thread_info)
                }
                PlaybackBackend::Cpal => cpal_control(
                    &path,
                    wav_secs,
                    decoded.expect("cpal exige WAV decodificado"),
                    ready_tx,
                    rx,
                    thread_playing,
                    thread_info,
                ),
            })
            .map_err(PlaybackError::ThreadSpawn)?;

        // Espera o stream tocar (ou o erro), como o `begin()` do capture.rs.
        // O `info` ja e publicado pela propria thread antes do `ready` (nao sobrescrever aqui).
        ready_rx.recv().map_err(|_| PlaybackError::ChannelClosed)??;

        Ok(Self { backend, tx, handle: Some(spawned), playing, info })
    }

    /// Nome do backend (para logs/relatorios).
    pub fn backend(&self) -> PlaybackBackend {
        self.backend
    }

    /// Metricas de inicio (clone; `first_sample_ms` pode ter sido preenchido depois pelo cpal).
    pub fn info(&self) -> Option<PlaybackInfo> {
        self.info.lock().expect("info mutex envenenado").clone()
    }

    /// True enquanto a thread de controle considera o audio tocando (stream vivo E samples
    /// restantes). Atualizacao com granularidade de `CONTROL_POLL` (50 ms) — suficiente pro spike.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Para o playback e devolve a latencia real do stop em ms (pergunta B do spike). Bloqueante:
    /// chamar fora de thread de UI/async (os comandos Tauri usam `spawn_blocking`).
    ///
    /// Semantica por backend (ver topo do arquivo):
    /// - Cpal: flag atomica (callback passa a escrever silencio) + drop do `cpal::Stream` (o
    ///   WASAPI encerra o stream). Latencia = ate o drop completar.
    /// - Rodio: `sink.stop()` + poll de `sink.empty()`. Latencia = ate o sink esvaziar
    ///   (SUBESTIMA o stop acustico; buffer do device nao e observavel).
    pub fn stop(&self) -> Result<StopReport, PlaybackError> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx
            .send(Command::Stop(reply_tx))
            // Canal fechado = a thread ja saiu (playback terminou naturalmente).
            .map_err(|_| PlaybackError::NotPlaying)?;
        reply_rx.recv().map_err(|_| PlaybackError::NotPlaying)?
    }
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                error!("thread audio-playback entrou em panico");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Backend Rodio
// ---------------------------------------------------------------------------

/// Thread de controle do rodio: abre `OutputStream` + `Sink`, anexa o WAV e fica no loop de
/// comandos. NOTA DE API (rodio 0.20, ver topo do arquivo): `OutputStream::try_default()` /
/// `Sink::try_new` e o padrao 0.17-0.19; se o 0.20 usar `OutputStreamBuilder`, ajustar AQUI.
fn rodio_control(
    path: &Path,
    wav_secs: f32,
    ready: Sender<Result<PlaybackInfo, PlaybackError>>,
    rx: Receiver<Command>,
    playing: Arc<AtomicBool>,
    info: Arc<Mutex<Option<PlaybackInfo>>>,
) {
    let t0 = Instant::now();

    let (stream, stream_handle) = match rodio::OutputStream::try_default() {
        Ok(v) => v,
        Err(e) => {
            let err = PlaybackError::OutputStream(e.to_string());
            error!(error = %err, "rodio: falha ao abrir OutputStream");
            let _ = ready.send(Err(err));
            return;
        }
    };
    let sink = match rodio::Sink::try_new(&stream_handle) {
        Ok(s) => s,
        Err(e) => {
            let err = PlaybackError::Sink(e.to_string());
            error!(error = %err, "rodio: falha ao criar Sink");
            let _ = ready.send(Err(err));
            return;
        }
    };

    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            let err = PlaybackError::Decode {
                path: path.display().to_string(),
                reason: e.to_string(),
            };
            error!(error = %err, "rodio: falha ao abrir o WAV");
            let _ = ready.send(Err(err));
            return;
        }
    };
    let source = match rodio::Decoder::new(BufReader::new(file)) {
        Ok(s) => s,
        Err(e) => {
            let err = PlaybackError::Decode {
                path: path.display().to_string(),
                reason: e.to_string(),
            };
            error!(error = %err, "rodio: falha ao decodificar o WAV");
            let _ = ready.send(Err(err));
            return;
        }
    };

    sink.append(source);
    let start_ms = t0.elapsed().as_millis() as u64;
    let start_info = PlaybackInfo { backend: PlaybackBackend::Rodio, wav_secs, start_ms, first_sample_ms: None };
    info!(
        backend = "rodio",
        wav_secs,
        start_ms,
        // first_sample_ms nao e observavel no rodio sem uma source customizada (ver topo).
        "playback iniciado (primeiro sample audivel = start_ms + ~1 periodo do device)"
    );
    playing.store(true, Ordering::Release);
    *info.lock().expect("info mutex envenenado") = Some(start_info.clone());
    if ready.send(Ok(start_info)).is_err() {
        return; // chamador desistiu (ex.: panic no play_wav); o Drop cuida do shutdown
    }

    loop {
        match rx.recv_timeout(CONTROL_POLL) {
            Ok(Command::Stop(reply)) => {
                let report = rodio_stop(&sink, &playing);
                let _ = reply.send(report);
                break;
            }
            Ok(Command::Shutdown) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // `sink.empty()`: true quando nao ha sources na fila. Fim natural do audio.
                let now_playing = !sink.empty();
                playing.store(now_playing, Ordering::Release);
                if !now_playing {
                    info!(backend = "rodio", wav_secs, "playback concluido (sink vazio)");
                    // Cauda: o sink esvazia antes do buffer do device; sem isso o drop corta ~10-20 ms.
                    thread::sleep(TAIL_GRACE);
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Drop explicito do stream: encerra o output device do rodio.
    drop(stream);
    playing.store(false, Ordering::Release);
}

/// Stop do rodio: `sink.stop()` + poll de `sink.empty()`. Ver aproximacao no topo do arquivo.
fn rodio_stop(sink: &rodio::Sink, playing: &Arc<AtomicBool>) -> Result<StopReport, PlaybackError> {
    let t0 = Instant::now();
    // rodio 0.20: confirmar se `stop()` segue existindo (0.17: limpa as fontes e para).
    sink.stop();

    let mut drained = false;
    while t0.elapsed() < RODIO_STOP_TIMEOUT {
        if sink.empty() {
            drained = true;
            break;
        }
        thread::sleep(RODIO_STOP_POLL);
    }
    playing.store(false, Ordering::Release);
    let latency = t0.elapsed().as_millis() as u64;
    if !drained {
        warn!(backend = "rodio", stop_latency_ms = latency, "sink nao esvaziou em 1 s");
    }
    info!(backend = "rodio", stop_latency_ms = latency, drained, "playback parado (aproximacao: sink vazio subestima o stop acustico)");
    Ok(StopReport {
        backend: PlaybackBackend::Rodio,
        stop_latency_ms: latency,
        drained,
        samples_undrained: None,
    })
}

// ---------------------------------------------------------------------------
// Backend Cpal
// ---------------------------------------------------------------------------

/// Estado vivo da thread de controle do cpal (contem o `Stream`, `!Send`).
struct CpalActive {
    /// `Option` para o stop poder derrubar o stream (drop = WASAPI encerra) e medir o tempo.
    stream: Option<cpal::Stream>,
    stop_flag: Arc<AtomicBool>,
    played: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
    first_flag: Arc<AtomicBool>,
    first_us: Arc<AtomicU64>,
    total: u64,
}

/// Thread de controle do cpal: abre o output stream consumindo o ringbuf e fica no loop de
/// comandos. O ringbuf ja vem preenchido com o WAV inteiro (resample feito antes), entao NAO ha
/// thread produtora: o callback e o unico consumidor (padrao mais simples que cabe no spike; no
/// M1-T10 o produtor sera a fila de frases do TTS).
fn cpal_control(
    path: &Path,
    wav_secs: f32,
    (samples, wav_rate): (Vec<f32>, u32),
    ready: Sender<Result<PlaybackInfo, PlaybackError>>,
    rx: Receiver<Command>,
    playing: Arc<AtomicBool>,
    info: Arc<Mutex<Option<PlaybackInfo>>>,
) {
    let t0 = Instant::now();

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            error!("cpal: nenhum dispositivo de saida encontrado");
            let _ = ready.send(Err(PlaybackError::NoOutputDevice));
            return;
        }
    };
    let supported = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            let err = PlaybackError::DeviceConfig(e.to_string());
            error!(error = %err, "cpal: falha na config do device de saida");
            let _ = ready.send(Err(err));
            return;
        }
    };
    let device_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let device_name = device.name().unwrap_or_else(|_| "<desconhecido>".into());
    info!(
        backend = "cpal",
        device = %device_name,
        sample_rate = device_rate,
        channels,
        format = ?supported.sample_format(),
        "device de saida escolhido"
    );
    if channels == 0 {
        let _ = ready.send(Err(PlaybackError::UnsupportedFormat("device com 0 canais".into())));
        return;
    }

    // Resample do WAV (22.05 kHz do Piper tipicamente) para a taxa do device. Sem isso o audio
    // toca acelerado/lento. Mesmo padrao do capture.rs (rubato FftFixedIn, mono).
    let resampled = match resample_to_device(samples, wav_rate, device_rate) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "cpal: falha no resample");
            let _ = ready.send(Err(e));
            return;
        }
    };
    let total = resampled.len() as u64;
    if total == 0 {
        let _ = ready.send(Err(PlaybackError::EmptyWav { path: path.display().to_string() }));
        return;
    }

    // Ringbuf SPSC com capacidade = WAV inteiro (mono f32). Cabe por construcao: produtor enche
    // tudo ANTES do stream iniciar e e descartado; o callback so consome. 10 s a 48 kHz = ~2 MB.
    let (mut producer, consumer) = HeapRb::<f32>::new(total as usize).split();
    producer.push_slice(&resampled);
    drop(producer);

    let played = Arc::new(AtomicU64::new(0));
    let underruns = Arc::new(AtomicU64::new(0));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let first_flag = Arc::new(AtomicBool::new(false));
    let first_us = Arc::new(AtomicU64::new(0));

    let config: cpal::StreamConfig = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_output_stream::<f32>(
            &device, &config, consumer, channels, total, &played, &stop_flag, &underruns,
            &first_flag, &first_us, t0,
        ),
        SampleFormat::I16 => build_output_stream::<i16>(
            &device, &config, consumer, channels, total, &played, &stop_flag, &underruns,
            &first_flag, &first_us, t0,
        ),
        SampleFormat::U16 => build_output_stream::<u16>(
            &device, &config, consumer, channels, total, &played, &stop_flag, &underruns,
            &first_flag, &first_us, t0,
        ),
        other => {
            let _ = ready.send(Err(PlaybackError::UnsupportedFormat(format!("{other:?}"))));
            return;
        }
    };
    let mut active = match stream {
        Ok(s) => {
            if let Err(e) = s.play() {
                let err = PlaybackError::PlayStream(e.to_string());
                error!(error = %err, "cpal: falha ao iniciar o stream de saida");
                let _ = ready.send(Err(err));
                return;
            }
            CpalActive { stream: Some(s), stop_flag, played, underruns, first_flag, first_us, total }
        }
        Err(e) => {
            error!(error = %e, "cpal: falha ao montar o stream de saida");
            let _ = ready.send(Err(e));
            return;
        }
    };

    let start_ms = t0.elapsed().as_millis() as u64;
    let start_info = PlaybackInfo {
        backend: PlaybackBackend::Cpal,
        wav_secs,
        start_ms,
        // Preenchido de verdade no primeiro callback (poll no loop abaixo).
        first_sample_ms: None,
    };
    info!(backend = "cpal", wav_secs, start_ms, total_samples = total, "playback iniciado");
    playing.store(true, Ordering::Release);
    *info.lock().expect("info mutex envenenado") = Some(start_info.clone());
    if ready.send(Ok(start_info)).is_err() {
        return;
    }

    loop {
        match rx.recv_timeout(CONTROL_POLL) {
            Ok(Command::Stop(reply)) => {
                let report = cpal_stop(&mut active, &playing);
                let _ = reply.send(report);
                break;
            }
            Ok(Command::Shutdown) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                publish_first_sample(&active, &info);
                let done = active.played.load(Ordering::Acquire) >= active.total;
                if done {
                    // Fim natural: encerra o stream e sai (mesma rota do stop, sem metrica de
                    // latencia — quem medir stop depois do fim recebe NotPlaying).
                    info!(
                        backend = "cpal",
                        underruns = active.underruns.load(Ordering::Relaxed),
                        "playback concluido"
                    );
                    thread::sleep(TAIL_GRACE);
                    break;
                }
                playing.store(!done, Ordering::Release);
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(active.stream.take());
    playing.store(false, Ordering::Release);
}

/// Preenche `first_sample_ms` nas metricas quando o primeiro callback do device rodar (so cpal).
fn publish_first_sample(active: &CpalActive, info: &Mutex<Option<PlaybackInfo>>) {
    if !active.first_flag.load(Ordering::Acquire) {
        return;
    }
    let mut guard = info.lock().expect("info mutex envenenado");
    if let Some(i) = guard.as_mut() {
        if i.first_sample_ms.is_none() {
            let us = active.first_us.load(Ordering::Relaxed);
            i.first_sample_ms = Some(us / 1000);
            info!(first_sample_ms = us / 1000, "primeiro callback do device (aproximacao do primeiro sample audivel)");
        }
    }
}

/// Stop do cpal: flag atomica (callback passa a escrever silencio) + drop do `cpal::Stream` (o
/// WASAPI encerra o stream). Latencia = pedido de stop ate o drop completar (ver topo do arquivo).
fn cpal_stop(active: &mut CpalActive, playing: &Arc<AtomicBool>) -> Result<StopReport, PlaybackError> {
    let t0 = Instant::now();
    // 1. Corta a alimentacao: no proximo callback (~1 periodo) o device recebe silencio.
    active.stop_flag.store(true, Ordering::Release);
    let remaining = active.total.saturating_sub(active.played.load(Ordering::Acquire));
    // 2. Derruba o stream: destroy do WASAPI. E o ponto mais proximo de "silencio de fato"
    //    observavel de dentro do Rust (o buffer interno da placa, 10-20 ms tipico em modo
    //    compartilhado, pode ainda soar por um instante — nao ha como sincronizar com ele).
    drop(active.stream.take());
    let latency = t0.elapsed().as_millis() as u64;
    playing.store(false, Ordering::Release);
    info!(
        backend = "cpal",
        stop_latency_ms = latency,
        samples_undrained = remaining,
        "playback parado (flag + drop do stream)"
    );
    Ok(StopReport {
        backend: PlaybackBackend::Cpal,
        stop_latency_ms: latency,
        drained: remaining == 0,
        samples_undrained: Some(remaining),
    })
}

/// Monta o stream de saida para o tipo de sample nativo `T`. O callback NAO aloca nem bloqueia:
/// pop no ringbuf (lock-free), expansao mono->N canais num buffer de stack e silencio no resto.
#[allow(clippy::too_many_arguments)]
fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut cons: HeapCons<f32>,
    channels: usize,
    total: u64,
    played: &Arc<AtomicU64>,
    stop_flag: &Arc<AtomicBool>,
    underruns: &Arc<AtomicU64>,
    first_flag: &Arc<AtomicBool>,
    first_us: &Arc<AtomicU64>,
    t0: Instant,
) -> Result<cpal::Stream, PlaybackError>
where
    T: SizedSample + cpal::FromSample<f32>, // espelho do bound de capture.rs (la `f32: FromSample<T>`)
{
    let played = Arc::clone(played);
    let stop_flag = Arc::clone(stop_flag);
    let underruns = Arc::clone(underruns);
    let first_flag = Arc::clone(first_flag);
    let first_us = Arc::clone(first_us);
    let mut scratch = [0f32; CALLBACK_SCRATCH];

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                // Primeiro callback = melhor aproximacao do primeiro sample audivel (barato: um
                // swap atomico, sem alocacao).
                if !first_flag.swap(true, Ordering::Relaxed) {
                    first_us.store(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
                }
                let silence = T::from_sample(0.0f32);
                // Stop pedido: silencio imediato, sem consumir o ringbuf (barge-in).
                if stop_flag.load(Ordering::Acquire) {
                    data.fill(silence);
                    return;
                }
                // O buffer do device pode ter mais frames que o scratch: processa em blocos.
                for block in data.chunks_mut(CALLBACK_SCRATCH * channels) {
                    let want = block.len() / channels;
                    let n = cons.pop_slice(&mut scratch[..want]);
                    let played_now = played.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                    // Expansao mono -> N canais (duplica o sample; spike, sem panorama estereo).
                    for (i, s) in scratch[..n].iter().enumerate() {
                        block[i * channels..(i + 1) * channels].fill(T::from_sample(*s));
                    }
                    // Silencio no resto: fim do audio ou underrun (nao deve acontecer — o ringbuf
                    // cabe o WAV inteiro — mas o contador diagnostica).
                    block[n * channels..].fill(silence);
                    if n < want && played_now < total {
                        underruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
            },
            // Callback de erro roda fora do caminho do audio; logar aqui e aceitavel.
            |e| error!(error = %e, "erro no stream de saida"),
            None,
        )
        .map_err(|e| PlaybackError::BuildStream(e.to_string()))
}

/// Resample mono (taxa do WAV) para a taxa do device. Mesmo padrao do capture.rs (rubato
/// FftFixedIn + cauda com `process_partial`).
fn resample_to_device(
    samples: Vec<f32>,
    from: u32,
    to: u32,
) -> Result<Vec<f32>, PlaybackError> {
    if from == to || samples.is_empty() {
        return Ok(samples);
    }
    let mut resampler = FftFixedIn::<f32>::new(
        from as usize,
        to as usize,
        RESAMPLE_CHUNK_FRAMES,
        2,
        1, // ja mono
    )
    .map_err(|e| PlaybackError::Resampler(e.to_string()))?;

    let mut pending = samples;
    let mut out: Vec<f32> = Vec::with_capacity(pending.len() * to as usize / from as usize + 1024);
    loop {
        let need = resampler.input_frames_next();
        if pending.len() < need {
            break;
        }
        let res = resampler
            .process(&[&pending[..need]], None)
            .map_err(|e| PlaybackError::Resampler(e.to_string()))?;
        out.extend_from_slice(&res[0]);
        pending.drain(..need);
    }
    // Cauda menor que um chunk: zero-padding (perde <~1 ms no fim do spike).
    if !pending.is_empty() {
        let res = resampler
            .process_partial(Some(&[&pending[..]]), None)
            .map_err(|e| PlaybackError::Resampler(e.to_string()))?;
        out.extend_from_slice(&res[0]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Leitura de WAV (hound) — compartilhado pelos dois backends
// ---------------------------------------------------------------------------

/// Duracao do WAV pelo header (frames / taxa). Erro tipado se o arquivo nao for WAV valido.
fn wav_duration_secs(path: &Path) -> Result<f32, PlaybackError> {
    let p = path.display().to_string();
    let reader = WavReader::open(path)
        .map_err(|e| PlaybackError::Wav { path: p, source: e })?;
    Ok(reader.duration() as f32 / reader.spec().sample_rate as f32)
}

/// Decodifica o WAV para mono f32 (normalizado -1..1) + taxa de origem. Suporta float 32-bit e
/// int 8/16/24/32-bit (o Piper escreve 16-bit PCM tipicamente). Estereo vira mono por media.
/// `Ok(None)` = WAV sem samples.
fn decode_wav_mono(path: &Path) -> Result<Option<(Vec<f32>, u32)>, PlaybackError> {
    let p = path.display().to_string();
    let mut reader = WavReader::open(path)
        .map_err(|e| PlaybackError::Wav { path: p.clone(), source: e })?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let mut mono: Vec<f32> = match spec.sample_format {
        HoundFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| PlaybackError::Wav { path: p.clone(), source: e })?,
        HoundFormat::Int => {
            let bits = spec.bits_per_sample;
            let mut buf: Vec<f32> = Vec::with_capacity(reader.duration() as usize * channels + 1);
            match bits {
                8 => {
                    for s in reader.samples::<i8>() {
                        buf.push(f32::from(s.map_err(|e| PlaybackError::Wav { path: p.clone(), source: e })?) / 128.0);
                    }
                }
                16 => {
                    for s in reader.samples::<i16>() {
                        buf.push(f32::from(s.map_err(|e| PlaybackError::Wav { path: p.clone(), source: e })?) / 32768.0);
                    }
                }
                24 | 32 => {
                    let scale = 2f32.powi(bits as i32 - 1);
                    for s in reader.samples::<i32>() {
                        buf.push(s.map_err(|e| PlaybackError::Wav { path: p.clone(), source: e })? as f32 / scale);
                    }
                }
                _ => return Err(PlaybackError::UnsupportedWavBits { path: p, bits }),
            }
            buf
        }
    };

    if mono.is_empty() {
        return Ok(None);
    }

    // Downmix estereo/multicanal -> mono (media dos canais), igual ao capture.rs.
    if channels > 1 {
        let frames = mono.len() / channels;
        for f in 0..frames {
            let sum: f32 = mono[f * channels..(f + 1) * channels].iter().sum();
            mono[f] = sum / channels as f32;
        }
        mono.truncate(frames);
    }

    Ok(Some((mono, spec.sample_rate)))
}

// ---------------------------------------------------------------------------
// Integracao Tauri (comandos do spike)
// ---------------------------------------------------------------------------

/// Estado gerenciado: o playback "atual" (o spike so toca uma coisa por vez; play novo derruba o
/// anterior via Drop -> Shutdown). `Arc<Mutex<_>>` para poder mover o lock para `spawn_blocking`
/// (os comandos sao async; as operacoes de audio sao bloqueantes — brainstorming 2.1).
#[derive(Clone, Default)]
pub struct PlaybackState {
    current: Arc<Mutex<Option<PlaybackHandle>>>,
}

/// Comando Tauri do spike: toca o WAV em `path` no backend escolhido e devolve as metricas de
/// inicio. O path vem do `synthesize_speech` (M0-T07: `res.wav_path`, em `./captures/tts/`).
#[tauri::command]
pub async fn play_wav_rust(
    path: String,
    backend: String,
    state: tauri::State<'_, PlaybackState>,
) -> Result<PlaybackInfo, String> {
    let current = Arc::clone(&state.current);
    tokio::task::spawn_blocking(move || {
        let backend = PlaybackBackend::parse(&backend).map_err(|e| e.to_string())?;
        let handle = PlaybackHandle::play_wav(Path::new(&path), backend)
            .map_err(|e| e.to_string())?;
        let info = handle.info().ok_or_else(|| "handle sem metricas de inicio".to_string())?;
        // Substitui o playback anterior (Drop -> Shutdown -> stream anterior encerra).
        if let Some(prev) = current.lock().expect("playback state mutex envenenado").replace(handle) {
            info!(backend = ?prev.backend(), "playback anterior substituido (Shutdown)");
        }
        Ok(info)
    })
    .await
    .map_err(|e| format!("task de playback falhou: {e}"))?
}

/// Comando Tauri do spike: para o playback atual e devolve `stop_latency_ms` (pergunta B).
#[tauri::command]
pub async fn stop_playback(state: tauri::State<'_, PlaybackState>) -> Result<StopReport, String> {
    let current = Arc::clone(&state.current);
    tokio::task::spawn_blocking(move || {
        let handle = current
            .lock()
            .expect("playback state mutex envenenado")
            .take()
            .ok_or(PlaybackError::NotPlaying)
            .map_err(|e| e.to_string())?;
        handle.stop().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task de stop falhou: {e}"))?
}

/// Relatorio do teste de conflito (pergunta A: cpal in + out simultaneos, WASAPI exclusivo?).
#[derive(Debug, Clone, Serialize)]
pub struct ConflictReport {
    pub backend: PlaybackBackend,
    /// Captura (mesmo caminho do PTT, `AudioCapture::start`) subiu?
    pub capture_ok: bool,
    pub capture_error: Option<String>,
    /// WAV da captura gravado no fim (prova de que a captura sobreviveu ao playback sem glitch
    /// reportado pelo cpal).
    pub capture_path: Option<String>,
    /// Playback subiu com a captura ja ativa?
    pub playback_ok: bool,
    pub playback_error: Option<String>,
    pub playback_info: Option<PlaybackInfo>,
    /// Se o playback ainda estava tocando no fim da janela de sobreposicao, stop medido.
    pub stop_report: Option<StopReport>,
    /// Janela em que in+out ficaram ativos juntos.
    pub overlap_secs: f64,
}

/// Comando Tauri do spike: teste de conflito in+out (plano M0-T08 item "verificar conflito de
/// device"). Inicia a captura, toca o WAV por cima com o backend escolhido, espera a duracao do
/// WAV + folga e encerra ambos, reportando se coexistiram. Tambem existe o teste manual (segurar
/// o PTT e clicar "Tocar via Rust") — ver docs/spikes.md.
#[tauri::command]
pub async fn spike_conflict_test(path: String, backend: String) -> Result<ConflictReport, String> {
    tokio::task::spawn_blocking(move || run_conflict_test(Path::new(&path), &backend))
        .await
        .map_err(|e| format!("task de conflito falhou: {e}"))?
}

fn run_conflict_test(path: &Path, backend_str: &str) -> Result<ConflictReport, String> {
    let backend = PlaybackBackend::parse(backend_str).map_err(|e| e.to_string())?;

    // 1. Captura primeiro (ordem do uso real: usuario pressiona PTT -> captura; resposta chega
    //    depois -> playback por cima).
    let (capture, capture_error) = match AudioCapture::start() {
        Ok(c) => (Some(c), None),
        Err(e) => {
            warn!(error = %e, "teste de conflito: captura nao iniciou (o teste segue so com playback)");
            (None, Some(e.to_string()))
        }
    };

    // 2. Playback com a captura (se) ativa.
    let (playback, playback_error) = match PlaybackHandle::play_wav(path, backend) {
        Ok(h) => {
            let info = h.info();
            (Some((h, info)), None)
        }
        Err(e) => {
            error!(error = %e, backend = backend_str, "teste de conflito: playback falhou");
            (None, Some(e.to_string()))
        }
    };
    let playback_info = playback.as_ref().and_then(|(_, i)| i.clone());

    // 3. Janela de sobreposicao: duracao do WAV + folga de 500 ms com ambos ativos.
    let wav_secs = wav_duration_secs(path).unwrap_or(2.0);
    let overlap = wav_secs + 0.5;
    thread::sleep(Duration::from_secs_f64(overlap.min(30.0)));

    // 4. Stop do playback (se ainda tocando) com a captura rodando: e o cenario real de barge-in.
    let stop_report = match playback.as_ref() {
        Some((h, _)) => match h.stop() {
            Ok(r) => Some(r),
            Err(PlaybackError::NotPlaying) => None, // ja tinha terminado naturalmente
            Err(e) => return Err(e.to_string()),
        },
        None => None,
    };
    drop(playback);

    // 5. Para a captura e grava o WAV: se o device aguentou os dois streams, a captura tem o
    //    audio do periodo inteiro (incluindo o proprio playback capturado pelo mic, se houver).
    let capture_path = match capture {
        Some(c) => match c.stop() {
            Ok(p) => Some(p.display().to_string()),
            Err(e) => {
                warn!(error = %e, "teste de conflito: captura falhou ao finalizar");
                None
            }
        },
        None => None,
    };

    let report = ConflictReport {
        backend,
        capture_ok: capture_error.is_none() && capture_path.is_some(),
        capture_error,
        capture_path,
        playback_ok: playback_error.is_none(),
        playback_error,
        playback_info,
        stop_report,
        overlap_secs: overlap,
    };
    info!(backend = ?report.backend, capture_ok = report.capture_ok, playback_ok = report.playback_ok, "teste de conflito in+out concluido");
    Ok(report)
}
