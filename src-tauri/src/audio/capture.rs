// Spike de captura de audio (M0-T05). Codigo de spike, descartavel exceto as medicoes (ver
// docs/spikes.md). Escopo: captura bruta -> mono -> 16 kHz -> WAV. SEM VAD, SEM FSM (M1/M5).
//
// Arquitetura (brainstorming 2.1, plano M0-T05):
//
//   [thread de controle]  dona do `cpal::Stream` (`!Send`: nao pode ir para `tauri::State` nem
//        |                para tasks Tokio). Recebe `Start`/`Stop`/`Shutdown` por canal.
//        v  cria
//   [callback do cpal] --(ringbuf SPSC, f32 intercalado, lock-free)--> [thread consumidora]
//    sem alocar/bloquear                                                downmix mono -> rubato 16 kHz
//                                                                       -> Vec<f32> acumulado
//   Ao `Stop`: a thread de controle derruba o stream, avisa o consumidor, junta o resultado e
//   escreve o WAV (16-bit PCM, 16 kHz, mono) em `<dir>/capture_<unix_ms>.wav`.
//
// Regra de canais (brainstorming 2.1): `crossbeam-channel` so entre threads de audio/controle;
// nada de `tokio::sync` aqui, e quem chama `stop()` de dentro de async deve usar `spawn_blocking`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use rubato::{FftFixedIn, Resampler};
use thiserror::Error;
use tracing::{error, info, warn};

/// Diretorio padrao dos WAVs do spike (relativo ao cwd do processo; no `tauri dev` o cwd e
/// `src-tauri/`). Esta no `.gitignore` tambem para o watcher do `tauri dev` nao reiniciar o app.
pub const DEFAULT_CAPTURE_DIR: &str = "./captures";

/// Formato alvo do Whisper (brainstorming 2.1): 16 kHz, mono.
const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Capacidade do ringbuf em segundos de audio nativo. Folga larga para o consumidor atrasar
/// (resample, escalonamento do SO) sem estourar; overrun e contado em `dropped_samples`.
const RING_SECONDS: usize = 4;

/// Frames de entrada por chunk do resampler (~21 ms a 48 kHz).
const RESAMPLE_CHUNK_FRAMES: usize = 1024;

/// Samples (f32) lidos do ringbuf por iteracao do consumidor.
const RAW_BUF_SAMPLES: usize = 8192;

/// Buffer de conversao no callback para formatos nao-f32 (alocado uma vez, na captura do closure).
const CALLBACK_SCRATCH: usize = 2048;

/// Intervalo de polling do consumidor quando o ringbuf esta vazio.
const CONSUMER_POLL: Duration = Duration::from_millis(5);

#[derive(Debug, Error)]
pub enum AudioCaptureError {
    #[error("nenhum dispositivo de entrada de audio encontrado")]
    NoInputDevice,
    #[error("falha ao consultar a configuracao do dispositivo: {0}")]
    DeviceConfig(String),
    #[error("formato de stream nao suportado: {0}")]
    UnsupportedFormat(String),
    #[error("falha ao abrir o stream de entrada: {0}")]
    BuildStream(String),
    #[error("falha ao iniciar o stream: {0}")]
    PlayStream(String),
    #[error("falha no resampler: {0}")]
    Resampler(String),
    #[error("erro de I/O em '{path}': {source}")]
    Io { path: String, source: io::Error },
    #[error("erro ao escrever WAV: {0}")]
    Wav(#[from] hound::Error),
    #[error("ja existe uma captura em andamento")]
    AlreadyCapturing,
    #[error("nenhuma captura em andamento")]
    NotCapturing,
    #[error("falha ao criar a thread de audio: {0}")]
    ThreadSpawn(io::Error),
    #[error("a thread de audio encerrou (canal fechado)")]
    ChannelClosed,
    #[error("a thread consumidora entrou em panico")]
    ConsumerPanicked,
}

/// Comandos da thread de controle (brainstorming 2.1: `Start`/`Stop`/`Shutdown`).
/// Cada `Start`/`Stop` leva um canal de resposta (one-shot) para o chamador esperar o resultado.
enum Command {
    Start(Sender<Result<(), AudioCaptureError>>),
    Stop(Sender<Result<PathBuf, AudioCaptureError>>),
    Shutdown,
}

/// Handle para a thread de audio. `Send` (so guarda canal + JoinHandle); o `cpal::Stream` `!Send`
/// nunca sai da thread de controle.
pub struct AudioCapture {
    tx: Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Spawna a thread de audio e inicia a captura no device de entrada padrao.
    /// WAVs vao para `DEFAULT_CAPTURE_DIR`.
    pub fn start() -> Result<Self, AudioCaptureError> {
        Self::start_in(DEFAULT_CAPTURE_DIR)
    }

    /// Igual a `start`, com diretorio de saida configuravel (criado se nao existir).
    pub fn start_in(dir: impl Into<PathBuf>) -> Result<Self, AudioCaptureError> {
        let capture = Self::spawn_idle(dir.into())?;
        capture.begin()?;
        Ok(capture)
    }

    /// Spawna a thread de controle sem iniciar o stream (use `begin` depois). Permite reutilizar
    /// a mesma thread em varios ciclos `begin`/`stop` (caminho da FSM no M1).
    pub fn spawn_idle(dir: PathBuf) -> Result<Self, AudioCaptureError> {
        let (tx, rx) = unbounded();
        let handle = thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || control_loop(rx, dir))
            .map_err(AudioCaptureError::ThreadSpawn)?;
        Ok(Self { tx, handle: Some(handle) })
    }

    /// Envia `Start` e espera o stream estar tocando (ou o erro de abertura do device).
    pub fn begin(&self) -> Result<(), AudioCaptureError> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx.send(Command::Start(reply_tx)).map_err(|_| AudioCaptureError::ChannelClosed)?;
        reply_rx.recv().map_err(|_| AudioCaptureError::ChannelClosed)?
    }

    /// Para a captura, escreve o WAV e devolve o caminho. Bloqueante (junta o consumidor e grava
    /// em disco): chamar fora de thread de UI/async.
    pub fn stop(&self) -> Result<PathBuf, AudioCaptureError> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx.send(Command::Stop(reply_tx)).map_err(|_| AudioCaptureError::ChannelClosed)?;
        reply_rx.recv().map_err(|_| AudioCaptureError::ChannelClosed)?
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        // Se ainda houver captura ativa, a thread de controle a descarta ao receber `Shutdown`.
        let _ = self.tx.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                error!("thread audio-capture entrou em panico");
            }
        }
    }
}

/// Info do device escolhido (para logs do go/no-go).
#[derive(Debug, Clone)]
struct DeviceInfo {
    name: String,
    sample_rate: u32,
    channels: u16,
    format: SampleFormat,
}

/// Captura em andamento. Vive so na thread de controle (contem o `Stream`, `!Send`).
struct Active {
    stream: cpal::Stream,
    stop_flag: Arc<AtomicBool>,
    consumer: JoinHandle<Result<ConsumerOutput, AudioCaptureError>>,
    dropped_samples: Arc<AtomicU64>,
    started: Instant,
    info: DeviceInfo,
}

/// Resultado do consumidor: audio ja em 16 kHz mono.
struct ConsumerOutput {
    samples: Vec<f32>,
    /// Frames (1 sample por canal) recebidos do device, antes do resample.
    native_frames: u64,
}

/// Loop da thread de controle. Dona do `Stream`; sai em `Shutdown` ou se todos os senders cairem.
fn control_loop(rx: Receiver<Command>, dir: PathBuf) {
    let mut active: Option<Active> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::Start(reply) => {
                let result = if active.is_some() {
                    Err(AudioCaptureError::AlreadyCapturing)
                } else {
                    open_capture().map(|a| active = Some(a))
                };
                if let Err(e) = &result {
                    error!(error = %e, "falha ao iniciar captura");
                }
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                let result = match active.take() {
                    None => Err(AudioCaptureError::NotCapturing),
                    Some(a) => finish_capture(a, &dir),
                };
                if let Err(e) = &result {
                    error!(error = %e, "falha ao finalizar captura");
                }
                let _ = reply.send(result);
            }
            Command::Shutdown => break,
        }
    }

    if let Some(a) = active.take() {
        warn!("encerrando com captura ativa: audio descartado");
        let Active { stream, stop_flag, consumer, .. } = a;
        drop(stream);
        stop_flag.store(true, Ordering::Release);
        let _ = consumer.join();
    }
}

/// Escolhe o device padrao, monta o stream + consumidor e da `play()`.
fn open_capture() -> Result<Active, AudioCaptureError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(AudioCaptureError::NoInputDevice)?;
    let supported = device
        .default_input_config()
        .map_err(|e| AudioCaptureError::DeviceConfig(e.to_string()))?;

    let info = DeviceInfo {
        name: device.name().unwrap_or_else(|_| "<desconhecido>".into()),
        sample_rate: supported.sample_rate().0,
        channels: supported.channels(),
        format: supported.sample_format(),
    };
    info!(
        device = %info.name,
        sample_rate = info.sample_rate,
        channels = info.channels,
        format = ?info.format,
        "device de entrada escolhido"
    );
    if info.channels == 0 {
        return Err(AudioCaptureError::UnsupportedFormat("device com 0 canais".into()));
    }

    // Resampler criado ANTES do stream: falha cedo, sem abrir o device a toa. Se o device ja e
    // 16 kHz, nao ha resample (so downmix).
    let resampler = if info.sample_rate == TARGET_SAMPLE_RATE {
        None
    } else {
        let r = FftFixedIn::<f32>::new(
            info.sample_rate as usize,
            TARGET_SAMPLE_RATE as usize,
            RESAMPLE_CHUNK_FRAMES,
            2,
            1, // ja mono depois do downmix
        )
        .map_err(|e| AudioCaptureError::Resampler(e.to_string()))?;
        Some(r)
    };

    // Ringbuf alocado aqui (fora do callback). SPSC: producer vai pro callback, consumer pra thread.
    let capacity = info.sample_rate as usize * info.channels as usize * RING_SECONDS;
    let (producer, consumer) = HeapRb::<f32>::new(capacity).split();
    let dropped_samples = Arc::new(AtomicU64::new(0));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let config: cpal::StreamConfig = supported.config();
    let stream = match info.format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, producer, &dropped_samples),
        SampleFormat::I16 => build_stream::<i16>(&device, &config, producer, &dropped_samples),
        SampleFormat::U16 => build_stream::<u16>(&device, &config, producer, &dropped_samples),
        SampleFormat::I32 => build_stream::<i32>(&device, &config, producer, &dropped_samples),
        other => return Err(AudioCaptureError::UnsupportedFormat(format!("{other:?}"))),
    }?;

    let channels = info.channels as usize;
    let consumer_stop = Arc::clone(&stop_flag);
    let consumer = thread::Builder::new()
        .name("audio-consumer".into())
        .spawn(move || consume(consumer, consumer_stop, channels, resampler))
        .map_err(AudioCaptureError::ThreadSpawn)?;

    if let Err(e) = stream.play() {
        // Consumidor ja esta de pe: manda parar e junta antes de devolver o erro.
        stop_flag.store(true, Ordering::Release);
        let _ = consumer.join();
        return Err(AudioCaptureError::PlayStream(e.to_string()));
    }

    info!("captura iniciada");
    Ok(Active { stream, stop_flag, consumer, dropped_samples, started: Instant::now(), info })
}

/// Monta o stream de entrada para o tipo de sample nativo `T`. O callback NAO aloca nem bloqueia:
/// converte para f32 num buffer de stack e faz `push_slice` no ringbuf (lock-free). Se o ringbuf
/// encher, descarta e conta (`dropped_samples`); nunca espera.
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut producer: HeapProd<f32>,
    dropped_samples: &Arc<AtomicU64>,
) -> Result<cpal::Stream, AudioCaptureError>
where
    T: SizedSample,
    f32: cpal::FromSample<T>,
{
    let dropped = Arc::clone(dropped_samples);
    let mut scratch = [0f32; CALLBACK_SCRATCH];

    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                for chunk in data.chunks(CALLBACK_SCRATCH) {
                    for (dst, src) in scratch.iter_mut().zip(chunk) {
                        *dst = f32::from_sample(*src);
                    }
                    let pushed = producer.push_slice(&scratch[..chunk.len()]);
                    if pushed < chunk.len() {
                        dropped.fetch_add((chunk.len() - pushed) as u64, Ordering::Relaxed);
                    }
                }
            },
            // Callback de erro roda fora do caminho do audio; logar aqui e aceitavel.
            |e| error!(error = %e, "erro no stream de entrada"),
            None,
        )
        .map_err(|e| AudioCaptureError::BuildStream(e.to_string()))
}

/// Thread consumidora: ringbuf -> downmix (media dos canais) -> resample -> acumula.
/// Termina quando `stop_flag` esta ativo E o ringbuf foi totalmente drenado.
fn consume(
    mut cons: HeapCons<f32>,
    stop_flag: Arc<AtomicBool>,
    channels: usize,
    mut resampler: Option<FftFixedIn<f32>>,
) -> Result<ConsumerOutput, AudioCaptureError> {
    let mut raw = vec![0f32; RAW_BUF_SAMPLES];
    // Samples de um frame incompleto (pop pode cortar no meio de um frame intercalado).
    let mut filled = 0usize;
    let mut pending: Vec<f32> = Vec::with_capacity(RESAMPLE_CHUNK_FRAMES * 2);
    let mut out: Vec<f32> = Vec::new();
    let mut native_frames = 0u64;

    loop {
        // Le a flag ANTES do pop: se ja era true, o stream ja foi derrubado e este pop ve tudo
        // que o callback escreveu; pop == 0 entao significa drenado de fato.
        let stopping = stop_flag.load(Ordering::Acquire);
        let n = cons.pop_slice(&mut raw[filled..]);
        if n == 0 {
            if stopping {
                break;
            }
            thread::sleep(CONSUMER_POLL);
            continue;
        }
        filled += n;

        let frames = filled / channels;
        let used = frames * channels;
        for frame in raw[..used].chunks_exact(channels) {
            pending.push(frame.iter().sum::<f32>() / channels as f32);
        }
        raw.copy_within(used..filled, 0);
        filled -= used;
        native_frames += frames as u64;

        resample_pending(&mut resampler, &mut pending, &mut out)?;
    }

    // Resto menor que um chunk: processa com zero-padding (cauda de <~21 ms).
    if let Some(r) = resampler.as_mut() {
        if !pending.is_empty() {
            let res = r
                .process_partial(Some(&[&pending[..]]), None)
                .map_err(|e| AudioCaptureError::Resampler(e.to_string()))?;
            out.extend_from_slice(&res[0]);
        }
    } else {
        out.append(&mut pending);
    }

    Ok(ConsumerOutput { samples: out, native_frames })
}

/// Consome `pending` (mono, taxa nativa) em chunks de `input_frames_next()` e anexa a `out` (16 kHz).
fn resample_pending(
    resampler: &mut Option<FftFixedIn<f32>>,
    pending: &mut Vec<f32>,
    out: &mut Vec<f32>,
) -> Result<(), AudioCaptureError> {
    let Some(r) = resampler.as_mut() else {
        // Device ja em 16 kHz: so passa adiante.
        out.append(pending);
        return Ok(());
    };
    loop {
        let need = r.input_frames_next();
        if pending.len() < need {
            return Ok(());
        }
        let res = r
            .process(&[&pending[..need]], None)
            .map_err(|e| AudioCaptureError::Resampler(e.to_string()))?;
        out.extend_from_slice(&res[0]);
        pending.drain(..need);
    }
}

/// Derruba o stream, drena o consumidor, loga as metricas e grava o WAV.
fn finish_capture(active: Active, dir: &Path) -> Result<PathBuf, AudioCaptureError> {
    let Active { stream, stop_flag, consumer, dropped_samples, started, info } = active;

    // Duracao real da captura (play -> stop), medida antes do teardown.
    let wall = started.elapsed();
    drop(stream);
    stop_flag.store(true, Ordering::Release);
    let output = consumer.join().map_err(|_| AudioCaptureError::ConsumerPanicked)??;

    let dropped = dropped_samples.load(Ordering::Relaxed);
    let wav_secs = output.samples.len() as f64 / TARGET_SAMPLE_RATE as f64;
    let native_secs = output.native_frames as f64 / info.sample_rate as f64;
    let (peak, rms) = level_stats(&output.samples);

    let path = write_wav(dir, &output.samples)?;

    info!(
        path = %path.display(),
        device = %info.name,
        native_rate = info.sample_rate,
        channels = info.channels,
        wall_s = wall.as_secs_f64(),
        native_s = native_secs,
        wav_s = wav_secs,
        native_frames = output.native_frames,
        wav_samples = output.samples.len(),
        dropped_samples = dropped,
        peak,
        rms,
        "captura finalizada"
    );
    if dropped > 0 {
        warn!(dropped_samples = dropped, "ringbuf estourou: consumidor nao acompanhou (buffer sizing?)");
    }
    if peak < 0.001 {
        warn!(peak, "audio praticamente silencioso: mic mudo ou device errado?");
    }
    Ok(path)
}

/// Pico absoluto e RMS (0.0..1.0) do audio final: ajuda a detectar silencio/clipping nos logs.
fn level_stats(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    (peak, rms)
}

/// Grava `samples` (f32, 16 kHz mono) como WAV 16-bit PCM em `dir/capture_<unix_ms>.wav`.
fn write_wav(dir: &Path, samples: &[f32]) -> Result<PathBuf, AudioCaptureError> {
    std::fs::create_dir_all(dir)
        .map_err(|source| AudioCaptureError::Io { path: dir.display().to_string(), source })?;

    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("capture_{unix_ms}.wav"));

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)?;
    }
    writer.finalize()?;
    Ok(path)
}
