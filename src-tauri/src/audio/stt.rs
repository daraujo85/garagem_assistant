// Spike STT (M0-T06): transcricao em lote de um WAV ja gravado com `whisper-rs` (whisper.cpp).
// Codigo de spike, descartavel exceto as medicoes (ver docs/spikes.md). Sem VAD e sem streaming por
// janelas: isso e M1/M5 (brainstorming 2.3). Aqui so se mede qualidade e tempo (RTF) em PT-BR.
//
// ATENCAO (API do whisper-rs): a crate muda bastante entre versoes. Este codigo foi escrito contra
// a API da 0.13 (`WhisperContext::new_with_params`, `create_state`, `state.full`,
// `full_n_segments`/`full_get_segment_text`/`full_get_segment_t0`). A partir da 0.14/0.15 os
// getters de segmento mudaram (iteradores / `WhisperSegment`). Confirmar no `cargo build` do Windows.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use serde::Serialize;
use thiserror::Error;
use tracing::{info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::capture::DEFAULT_CAPTURE_DIR;

/// Caminho padrao do modelo ggml (relativo ao cwd; no `tauri dev` o cwd e `src-tauri/`).
/// A pasta `models/` esta no `.gitignore`. Sobrescrevivel com a env `GARAGEM_STT_MODEL`
/// (ex.: `./models/ggml-small-q5_1.bin`). M2: via preferences.
pub const DEFAULT_MODEL_PATH: &str = "./models/ggml-base.bin";
pub const MODEL_ENV: &str = "GARAGEM_STT_MODEL";

/// Idioma forcado (brainstorming 2.3: PT-BR; forcar evita o custo e os erros do auto-detect).
const LANGUAGE: &str = "pt";

/// Vocabulario dev usado como `initial_prompt` (brainstorming 2.3: PT-BR com termos tecnicos em
/// ingles; o prompt enviesa o decoder a grafar esses termos corretamente, reduzindo erro neles).
/// Parte do que o spike mede: comparar com e sem prompt (plano M0-T06).
const INITIAL_PROMPT: &str = "git, commit, branch, merge, pull request, funcao, variavel, deploy, \
build, terminal, codigo, refatorar, teste, log, erro";

/// O Whisper exige 16 kHz mono (brainstorming 2.1); o capture.rs ja gera nesse formato.
const EXPECTED_SAMPLE_RATE: u32 = 16_000;

/// Abaixo disso a captura seria descartada em producao (brainstorming 2.3: <300 ms). No spike
/// so avisamos, para ainda poder medir.
const MIN_AUDIO_SECS: f32 = 0.3;

/// Teto de threads de inferencia em CPU: mais que isso rende pouco no whisper.cpp.
const MAX_THREADS: usize = 8;

#[derive(Debug, Error)]
pub enum SttError {
    #[error(
        "modelo Whisper nao encontrado em '{0}'. Baixe um modelo ggml (ex.: ggml-base.bin ou \
         ggml-small-q5_1.bin) de https://huggingface.co/ggerganov/whisper.cpp/tree/main e coloque \
         em ./models/ (ou aponte a env GARAGEM_STT_MODEL). Ver docs/spikes.md"
    )]
    ModelNotFound(String),
    #[error("falha ao carregar o modelo '{path}': {reason}")]
    ModelLoad { path: String, reason: String },
    #[error("falha ao ler o WAV '{path}': {source}")]
    Wav { path: String, source: hound::Error },
    #[error("WAV em formato inesperado: {0}")]
    WavFormat(String),
    #[error("falha na inferencia do Whisper: {0}")]
    Inference(String),
    #[error("nenhuma captura (.wav) encontrada em '{0}'. Grave com o atalho PTT primeiro")]
    NoCapture(String),
    #[error("erro de I/O em '{path}': {source}")]
    Io { path: String, source: std::io::Error },
}

/// Trecho transcrito com timestamps (whisper.cpp devolve em centissegundos; convertemos p/ ms).
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Resultado de uma transcricao + as metricas que o spike precisa (plano M0-T06).
#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionResult {
    pub text: String,
    /// Idioma FORCADO ("pt"); nao e detectado (ver `LANGUAGE`).
    pub language: String,
    pub audio_secs: f32,
    /// Tempo so da inferencia (`state.full` + leitura dos segmentos). Nao inclui ler o WAV nem
    /// carregar o modelo.
    pub processing_ms: u64,
    /// Realtime factor = processing_ms / (audio_secs * 1000). < 1.0 = mais rapido que tempo real.
    pub rtf: f32,
    pub segments: Vec<Segment>,
    /// Nome do arquivo do modelo usado (para a tabela de resultados).
    pub model: String,
    pub wav_path: String,
}

/// STT em lote. O modelo e carregado UMA vez (cold start de segundos; brainstorming 2.4) e
/// reaproveitado: `WhisperContext` e `Send + Sync`, entao um `Arc<WhisperStt>` serve.
pub struct WhisperStt {
    ctx: WhisperContext,
    model_name: String,
    n_threads: i32,
}

impl WhisperStt {
    /// Carrega o modelo de `model_path`. Erro claro (com onde baixar) se o arquivo nao existir.
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, SttError> {
        let path = model_path.as_ref();
        if !path.is_file() {
            return Err(SttError::ModelNotFound(path.display().to_string()));
        }
        let path_str = path.to_str().ok_or_else(|| SttError::ModelLoad {
            path: path.display().to_string(),
            reason: "caminho nao e UTF-8 valido".into(),
        })?;

        let t0 = Instant::now();
        // Com a feature `cuda` do whisper-rs compilada, o default de `use_gpu` ja usa a GPU; sem
        // ela roda em CPU. Nao forcamos nada aqui (M0 testa CPU primeiro, depois GPU).
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| SttError::ModelLoad {
                path: path_str.to_string(),
                reason: e.to_string(),
            })?;

        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(MAX_THREADS) as i32;
        let model_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_str.to_string());

        info!(
            model = %model_name,
            load_ms = t0.elapsed().as_millis() as u64,
            n_threads,
            "modelo Whisper carregado"
        );
        Ok(Self { ctx, model_name, n_threads })
    }

    /// Transcreve um WAV 16 kHz mono (formato gerado por `audio::capture`). Bloqueante e
    /// CPU-bound: chamar de `spawn_blocking`, nunca direto de uma `async fn` (brainstorming 2.1).
    pub fn transcribe(&self, wav_path: &Path) -> Result<TranscriptionResult, SttError> {
        let samples = read_wav_mono_16k(wav_path)?;
        let audio_secs = samples.len() as f32 / EXPECTED_SAMPLE_RATE as f32;
        if audio_secs < MIN_AUDIO_SECS {
            warn!(audio_secs, "audio muito curto (<300 ms); em producao seria descartado (VAD, M1)");
        }

        // Estado novo por chamada: simples e sem estado vazado entre transcricoes. O custo de
        // alocar o estado entra no `processing_ms` (e pequeno perto da inferencia).
        let t0 = Instant::now();
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| SttError::Inference(e.to_string()))?;

        // Greedy (best_of 1) = mais rapido; beam search so se a qualidade nao bastar.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(LANGUAGE));
        params.set_translate(false);
        params.set_initial_prompt(INITIAL_PROMPT);
        params.set_n_threads(self.n_threads);
        // Sem contexto de chamadas anteriores: cada captura e um turno independente.
        params.set_no_context(true);
        // O whisper.cpp imprime em stdout por padrao; o log vai por `tracing`.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, &samples)
            .map_err(|e| SttError::Inference(e.to_string()))?;

        let n_segments = state
            .full_n_segments()
            .map_err(|e| SttError::Inference(e.to_string()))?;
        let mut segments = Vec::with_capacity(n_segments.max(0) as usize);
        let mut text = String::new();
        for i in 0..n_segments {
            let seg_text = state
                .full_get_segment_text(i)
                .map_err(|e| SttError::Inference(e.to_string()))?;
            // t0/t1 em centissegundos (10 ms).
            let t_start = state.full_get_segment_t0(i).unwrap_or(0);
            let t_end = state.full_get_segment_t1(i).unwrap_or(0);
            text.push_str(&seg_text);
            segments.push(Segment {
                start_ms: t_start * 10,
                end_ms: t_end * 10,
                text: seg_text.trim().to_string(),
            });
        }
        let text = text.trim().to_string();

        let processing_ms = t0.elapsed().as_millis() as u64;
        let rtf = if audio_secs > 0.0 {
            processing_ms as f32 / (audio_secs * 1000.0)
        } else {
            0.0
        };

        // Spike: log do texto e aceitavel aqui para validar a qualidade; producao (M1+) nao loga
        // transcricao por padrao (privacidade, brainstorming).
        info!(
            model = %self.model_name,
            audio_secs,
            processing_ms,
            rtf,
            segments = segments.len(),
            text = %text,
            "transcricao concluida"
        );

        Ok(TranscriptionResult {
            text,
            language: LANGUAGE.to_string(),
            audio_secs,
            processing_ms,
            rtf,
            segments,
            model: self.model_name.clone(),
            wav_path: wav_path.display().to_string(),
        })
    }
}

/// Le o WAV e devolve f32 em [-1, 1]. Exige 16 kHz mono (o que o capture.rs gera): sem resample
/// aqui de proposito, um WAV fora do formato indica bug na captura e deve aparecer como erro.
fn read_wav_mono_16k(path: &Path) -> Result<Vec<f32>, SttError> {
    let wav_err = |source| SttError::Wav { path: path.display().to_string(), source };
    let mut reader = hound::WavReader::open(path).map_err(wav_err)?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != EXPECTED_SAMPLE_RATE {
        return Err(SttError::WavFormat(format!(
            "esperado 16000 Hz mono, obtido {} Hz, {} canal(is)",
            spec.sample_rate, spec.channels
        )));
    }

    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()
            .map_err(wav_err)?,
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(wav_err)?,
        (fmt, bits) => {
            return Err(SttError::WavFormat(format!(
                "formato de sample nao suportado: {fmt:?} {bits} bits"
            )))
        }
    };
    Ok(samples)
}

/// WAV mais recente (por mtime) em `dir`.
fn latest_wav(dir: &Path) -> Result<PathBuf, SttError> {
    let io_err = |source| SttError::Io { path: dir.display().to_string(), source };
    let entries = fs::read_dir(dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SttError::NoCapture(dir.display().to_string())
        } else {
            io_err(e)
        }
    })?;

    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        let is_wav = p
            .extension()
            .map(|e| e.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);
        if !is_wav {
            continue;
        }
        let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if best.as_ref().map_or(true, |(t, _)| mtime > *t) {
            best = Some((mtime, p));
        }
    }
    best.map(|(_, p)| p)
        .ok_or_else(|| SttError::NoCapture(dir.display().to_string()))
}

/// Helper do spike: acha o WAV mais recente em `./captures/` e transcreve.
pub fn transcribe_latest(stt: &WhisperStt) -> Result<TranscriptionResult, SttError> {
    let wav = latest_wav(Path::new(DEFAULT_CAPTURE_DIR))?;
    info!(wav = %wav.display(), "transcrevendo a captura mais recente");
    stt.transcribe(&wav)
}

/// Estado Tauri do spike: o modelo e carregado de forma lazy na primeira chamada e fica
/// residente (a RAM residente e uma das medicoes do M0-T06). Em M1 o pre-load vai pro boot
/// (brainstorming 2.4: cold start). `Clone` barato (so `Arc`) para entrar no `spawn_blocking`.
#[derive(Clone, Default)]
pub struct SttState {
    inner: Arc<Mutex<Option<Arc<WhisperStt>>>>,
}

impl SttState {
    /// Devolve o modelo carregado, carregando-o se ainda nao estiver. Bloqueante (carga do modelo).
    fn get_or_load(&self) -> Result<Arc<WhisperStt>, SttError> {
        // Mutex envenenado so ocorre se a carga anterior entrou em panico; seguimos com o valor.
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stt) = guard.as_ref() {
            return Ok(Arc::clone(stt));
        }
        let path = std::env::var(MODEL_ENV).unwrap_or_else(|_| DEFAULT_MODEL_PATH.to_string());
        let stt = Arc::new(WhisperStt::load(path)?);
        *guard = Some(Arc::clone(&stt));
        Ok(stt)
    }
}

/// Comando Tauri do spike: transcreve o WAV mais recente de `./captures/`. Chamado manualmente
/// pelo botao do frontend para medir sem script separado. A inferencia e bloqueante e CPU-bound,
/// entao roda em `spawn_blocking` (brainstorming 2.1). O modelo (re)usado fica no `SttState`.
#[tauri::command]
pub async fn transcribe_last_capture(
    state: tauri::State<'_, SttState>,
) -> Result<TranscriptionResult, String> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let stt = state.get_or_load()?;
        transcribe_latest(&stt)
    })
    .await
    .map_err(|e| format!("a thread de transcricao falhou: {e}"))?
    .map_err(|e| e.to_string())
}
