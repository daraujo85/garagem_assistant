// Spike TTS (M0-T07): sintese de voz com o Piper como BINARIO SIDECAR (processo externo), nao como
// crate Rust. Motivo: nao ha binding Rust maduro para o Piper (ao contrario do `whisper-rs`) e o
// sidecar e a integracao mais robusta (brainstorming 2.9). Codigo de spike, descartavel exceto as
// medicoes (ver docs/spikes.md). Sem streaming de PCM (`--output-raw`) e sem fila/playback em Rust:
// isso e M1 (plano M0-T08 faz o playback). Aqui so se gera um WAV e mede o tempo.
//
// Fluxo: `piper --model <voz>.onnx --output_file <saida>.wav` com o texto no stdin. O arquivo
// `<voz>.onnx.json` (config da voz) tem que estar ao lado do `.onnx`. As flags acima sao as do
// binario classico (rhasspy/piper, release com `piper.exe`). Se o binario escolhido for o fork
// `OHF-Voice/piper1-gpl`, as flags podem diferir (ex.: `--output-file`/`-f`): ajustar `build_args`.
//
// Escolha async: `tokio::process::Command` (e nao `spawn_blocking` + `std::process`). O trabalho
// pesado roda no OUTRO processo, entao nao ha CPU-bound dentro do Rust e nenhuma thread fica
// bloqueada esperando: o `await` so suspende a task (brainstorming 2.1). A leitura do header do WAV
// (`hound`) e O(1) e pode ficar inline.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{info, warn};

/// Binario do Piper (relativo ao cwd; no `tauri dev` o cwd e `src-tauri/`). A pasta `tools/` esta
/// no `.gitignore`. Sobrescrevivel com a env `GARAGEM_PIPER_BIN` (caminho completo ou nome no PATH).
#[cfg(windows)]
pub const DEFAULT_BIN_PATH: &str = "./tools/piper/piper.exe";
#[cfg(not(windows))]
pub const DEFAULT_BIN_PATH: &str = "./tools/piper/piper";
pub const BIN_ENV: &str = "GARAGEM_PIPER_BIN";

/// Voz padrao (`.onnx`; o `.onnx.json` fica ao lado). `models/` esta no `.gitignore`.
/// Sobrescrevivel com `GARAGEM_PIPER_VOICE` (ex.: `./models/voices/pt_BR-edresson-low.onnx`).
/// Brainstorming 2.9: vozes PT-BR existentes sao faber e edresson, qualidade mediana.
pub const DEFAULT_VOICE_PATH: &str = "./models/voices/pt_BR-faber-medium.onnx";
pub const VOICE_ENV: &str = "GARAGEM_PIPER_VOICE";

/// Pasta de saida dos WAVs gerados. Dentro de `captures/` de proposito: ja e ignorada pelo git e
/// pelo watcher do `tauri dev` (gravar um WAV ali nao pode reiniciar o app). O `latest_wav` do
/// stt.rs nao e recursivo, entao estes arquivos nao poluem a busca da ultima captura.
pub const OUTPUT_DIR: &str = "./captures/tts";

/// Intervalo de polling do arquivo de saida para aproximar o "primeiro byte" (ver `synthesize`).
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Teto de tempo de uma sintese. Evita travar o spike se o Piper pendurar.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Quanto do stderr do Piper entra na mensagem de erro (o Piper e verboso).
const STDERR_TAIL_CHARS: usize = 600;

/// Contador para nomes de arquivo unicos dentro do mesmo milissegundo.
static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum TtsError {
    #[error(
        "binario do Piper nao encontrado em '{0}'. Baixe o release para Windows (piper_windows_amd64.zip) \
         em https://github.com/rhasspy/piper/releases (ou o fork ativo em \
         https://github.com/OHF-Voice/piper1-gpl) e extraia em ./tools/piper/ (ou aponte a env \
         GARAGEM_PIPER_BIN). Ver docs/spikes.md"
    )]
    BinaryNotFound(String),
    #[error(
        "voz do Piper nao encontrada: '{0}'. Baixe o .onnx E o .onnx.json da voz (ex.: \
         pt_BR-faber-medium ou pt_BR-edresson-low) em https://huggingface.co/rhasspy/piper-voices \
         (pasta pt/pt_BR/) e coloque em ./models/voices/ (ou aponte a env GARAGEM_PIPER_VOICE). \
         Ver docs/spikes.md"
    )]
    VoiceNotFound(String),
    #[error("texto vazio: nada para sintetizar")]
    EmptyText,
    #[error("o Piper falhou ({status}): {stderr}")]
    ProcessFailed { status: String, stderr: String },
    #[error("o Piper excedeu o tempo limite de {0} s e foi encerrado")]
    Timeout(u64),
    #[error("erro de I/O em '{path}': {source}")]
    Io { path: String, source: std::io::Error },
    #[error("falha ao ler o WAV '{path}': {source}")]
    Wav { path: String, source: hound::Error },
}

/// Resultado de uma sintese + as metricas que o spike precisa (plano M0-T07).
#[derive(Debug, Clone, Serialize)]
pub struct SynthesisResult {
    /// Texto de entrada (eco, para montar a tabela de resultados).
    pub text: String,
    /// Caminho do WAV gerado (usar `read_synthesized_wav` para obter os bytes no frontend).
    pub wav_path: String,
    /// Duracao do audio gerado, lida do header do WAV (`hound`).
    pub audio_secs: f32,
    pub sample_rate: u32,
    /// Tempo total: do spawn do processo ate ele terminar (inclui carga do modelo ONNX, que no
    /// binario sidecar acontece A CADA chamada: custo que o streaming/processo residente do M1
    /// precisa eliminar). Nao inclui a leitura do header do WAV.
    pub processing_ms: u64,
    /// APROXIMACAO do tempo ate o primeiro byte no arquivo de saida (polling a cada 5 ms; erro de
    /// ate ~5 ms). LIMITACAO: o Piper em modo `--output_file` pode criar o arquivo no inicio (header
    /// vazio) ou so gravar no final; nao ha como distinguir de fora. `None` = o arquivo so apareceu
    /// com conteudo depois do fim do processo. Nao e o "first chunk" real do streaming (brainstorming
    /// 2.4: <200 ms), que so se mede em M1 com `--output-raw`.
    pub first_output_ms: Option<u64>,
    /// `processing_ms / (audio_secs * 1000)`. < 1.0 = sintetiza mais rapido que tempo real.
    pub rtf: f32,
    /// Nome do arquivo da voz usada (para a tabela de resultados).
    pub voice: String,
}

/// TTS via Piper sidecar. Guarda so os caminhos: o "carregamento" do modelo acontece dentro do
/// processo Piper a cada chamada. A struct e barata de criar, entao o comando a cria por chamada
/// a partir da env (sem `State` gerenciado, diferente do `SttState` do Whisper).
#[derive(Debug, Clone)]
pub struct PiperTts {
    bin_path: PathBuf,
    voice_path: PathBuf,
}

impl PiperTts {
    /// Valida que o binario e a voz (`.onnx` + `.onnx.json`) existem. Erro claro, com onde baixar.
    pub fn new(
        bin_path: impl Into<PathBuf>,
        voice_path: impl Into<PathBuf>,
    ) -> Result<Self, TtsError> {
        let bin_path = bin_path.into();
        let voice_path = voice_path.into();

        // Nome sem separador (ex.: "piper") vem do PATH: nao da pra checar com `is_file`; se nao
        // existir, o `spawn` devolve NotFound e vira `BinaryNotFound` em `synthesize`.
        let has_dir = bin_path.components().count() > 1;
        if has_dir && !bin_path.is_file() {
            return Err(TtsError::BinaryNotFound(bin_path.display().to_string()));
        }
        if !voice_path.is_file() {
            return Err(TtsError::VoiceNotFound(voice_path.display().to_string()));
        }
        let config = voice_config_path(&voice_path);
        if !config.is_file() {
            return Err(TtsError::VoiceNotFound(format!(
                "{} (falta o arquivo de config {})",
                voice_path.display(),
                config.display()
            )));
        }
        Ok(Self { bin_path, voice_path })
    }

    /// Le `GARAGEM_PIPER_BIN` / `GARAGEM_PIPER_VOICE`, com os defaults acima. M2: via preferences.
    pub fn from_env() -> Result<Self, TtsError> {
        let bin = std::env::var(BIN_ENV).unwrap_or_else(|_| DEFAULT_BIN_PATH.to_string());
        let voice = std::env::var(VOICE_ENV).unwrap_or_else(|_| DEFAULT_VOICE_PATH.to_string());
        Self::new(bin, voice)
    }

    /// Argumentos do Piper para a versao classica (rhasspy/piper). Isolado para facilitar o ajuste
    /// se o binario do fork piper1-gpl usar flags diferentes.
    fn build_args(&self, output: &Path) -> Vec<std::ffi::OsString> {
        vec![
            "--model".into(),
            self.voice_path.as_os_str().to_owned(),
            "--output_file".into(),
            output.as_os_str().to_owned(),
        ]
    }

    /// Sintetiza `text` em um WAV novo dentro de `OUTPUT_DIR`. Texto via stdin; espera o processo
    /// terminar; le a duracao do header do WAV. Mede o tempo total e aproxima o primeiro byte.
    pub async fn synthesize(&self, text: &str) -> Result<SynthesisResult, TtsError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(TtsError::EmptyText);
        }

        let out_dir = Path::new(OUTPUT_DIR);
        tokio::fs::create_dir_all(out_dir)
            .await
            .map_err(|source| TtsError::Io { path: OUTPUT_DIR.to_string(), source })?;
        let out_path = out_dir.join(unique_wav_name());

        let mut cmd = Command::new(&self.bin_path);
        cmd.args(self.build_args(&out_path))
            .stdin(Stdio::piped())
            // O stdout nao e usado no modo --output_file; descartar evita encher o pipe.
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            // Se o future for cancelado/timeout, o processo nao fica orfao.
            .kill_on_drop(true);
        // Windows: sem isto o piper.exe abre uma janela de console a cada chamada.
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let bin_display = self.bin_path.display().to_string();
        let t0 = Instant::now();
        let mut child = cmd.spawn().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                TtsError::BinaryNotFound(bin_display.clone())
            } else {
                TtsError::Io { path: bin_display.clone(), source }
            }
        })?;

        // stderr e lido em task propria: se o Piper escrever muito e ninguem ler, o pipe enche e o
        // processo trava (deadlock) antes de terminar.
        let mut stderr = child.stderr.take().expect("stderr foi configurado como piped");
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        });

        // Escreve o texto e FECHA o stdin (drop): o Piper so sintetiza ao ver EOF. Termina com
        // '\n' porque o Piper processa uma linha = uma utterance.
        let mut stdin = child.stdin.take().expect("stdin foi configurado como piped");
        let payload = format!("{text}\n");
        if let Err(source) = stdin.write_all(payload.as_bytes()).await {
            // Broken pipe = o Piper morreu cedo (ex.: voz invalida); o erro real esta no stderr.
            warn!(error = %source, "falha ao escrever o texto no stdin do Piper");
        }
        drop(stdin);

        // Espera o fim com polling do arquivo de saida (aproximacao do primeiro byte).
        let mut first_output_ms: Option<u64> = None;
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        let deadline = tokio::time::sleep(TIMEOUT);
        tokio::pin!(deadline);
        let status = loop {
            tokio::select! {
                res = child.wait() => {
                    break res.map_err(|source| TtsError::Io { path: bin_display.clone(), source })?;
                }
                _ = tick.tick(), if first_output_ms.is_none() => {
                    if let Ok(meta) = tokio::fs::metadata(&out_path).await {
                        if meta.len() > 0 {
                            first_output_ms = Some(t0.elapsed().as_millis() as u64);
                        }
                    }
                }
                _ = &mut deadline => {
                    let _ = child.kill().await;
                    return Err(TtsError::Timeout(TIMEOUT.as_secs()));
                }
            }
        };
        let processing_ms = t0.elapsed().as_millis() as u64;

        let stderr_text = stderr_task.await.unwrap_or_default();
        if !status.success() {
            return Err(TtsError::ProcessFailed {
                status: status.to_string(),
                stderr: tail(&stderr_text, STDERR_TAIL_CHARS),
            });
        }

        let (audio_secs, sample_rate) = wav_duration(&out_path)?;
        let rtf = if audio_secs > 0.0 {
            processing_ms as f32 / (audio_secs * 1000.0)
        } else {
            0.0
        };
        let voice = self
            .voice_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.voice_path.display().to_string());

        // Spike: logar o texto e aceitavel; producao (M1+) nao loga conteudo por padrao.
        info!(
            voice = %voice,
            chars = text.chars().count(),
            audio_secs,
            processing_ms,
            first_output_ms = ?first_output_ms,
            rtf,
            "sintese concluida"
        );

        Ok(SynthesisResult {
            text: text.to_string(),
            wav_path: out_path.display().to_string(),
            audio_secs,
            sample_rate,
            processing_ms,
            first_output_ms,
            rtf,
            voice,
        })
    }
}

/// `pt_BR-faber-medium.onnx` -> `pt_BR-faber-medium.onnx.json` (convencao do Piper).
fn voice_config_path(voice: &Path) -> PathBuf {
    let mut s = voice.as_os_str().to_owned();
    s.push(".json");
    PathBuf::from(s)
}

/// Nome unico: timestamp em ms + contador (duas sinteses no mesmo ms nao colidem).
fn unique_wav_name() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("tts_{ms}_{n}.wav")
}

/// Duracao (s) e sample rate lidos so do header do WAV (nao decodifica as amostras).
fn wav_duration(path: &Path) -> Result<(f32, u32), TtsError> {
    let reader = hound::WavReader::open(path)
        .map_err(|source| TtsError::Wav { path: path.display().to_string(), source })?;
    let spec = reader.spec();
    // `duration()` = amostras por canal.
    let secs = reader.duration() as f32 / spec.sample_rate as f32;
    Ok((secs, spec.sample_rate))
}

/// Ultimos `max` caracteres (as mensagens de erro do Piper ficam no fim do stderr).
fn tail(s: &str, max: usize) -> String {
    let s = s.trim();
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let skip = count - max;
        format!("...{}", s.chars().skip(skip).collect::<String>())
    }
}

/// Comando Tauri do spike: sintetiza `text` e devolve metricas + caminho do WAV. Async com
/// `tokio::process` (ver nota no topo do arquivo): nao precisa de `spawn_blocking`.
#[tauri::command]
pub async fn synthesize_speech(text: String) -> Result<SynthesisResult, String> {
    let tts = PiperTts::from_env().map_err(|e| e.to_string())?;
    tts.synthesize(&text).await.map_err(|e| e.to_string())
}

/// Comando Tauri do spike: devolve os BYTES do WAV gerado como resposta binaria crua (chega no JS
/// como `ArrayBuffer`, sem JSON/base64). O frontend monta um `Blob` + `URL.createObjectURL` para a
/// tag `<audio>`. Evita habilitar o asset protocol so para o spike; playback em Rust e M0-T08.
/// So le arquivos dentro de `OUTPUT_DIR` (nao vira leitor generico de disco para o webview).
#[tauri::command]
pub async fn read_synthesized_wav(path: String) -> Result<tauri::ipc::Response, String> {
    let base = tokio::fs::canonicalize(OUTPUT_DIR)
        .await
        .map_err(|e| format!("pasta de saida do TTS indisponivel: {e}"))?;
    let file = tokio::fs::canonicalize(&path)
        .await
        .map_err(|e| format!("WAV '{path}' indisponivel: {e}"))?;
    if !file.starts_with(&base) {
        return Err(format!("caminho fora da pasta de saida do TTS: '{path}'"));
    }
    let bytes = tokio::fs::read(&file)
        .await
        .map_err(|e| format!("falha ao ler '{path}': {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}
