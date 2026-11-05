// Spike do gateway (M0-T11): cliente HTTP/SSE para o 9router. SO conectividade e medicao de latencia;
// NAO e o roteamento/dispatcher (esse depende de D5/D6, ver dispatcher.rs) e NAO e o `LlmGateway`
// final do M1-T07 (cancelamento, timeout por estagio, hook de egress). Este arquivo prepara o M1-T07:
// o formato da chamada (OpenAI-compativel + SSE) e a forma de medir first token ja ficam validados.
//
// Contexto: brainstorming 2.4 (dois hops de LLM dobram o custo; medir desde o dia 1, o first token
// precisa caber em 300-1500 ms) e 2.8 (9router e um gateway HTTP em localhost:20128, com combos e
// fallback proprios; precisamos saber o que acontece com ele fora do ar). Plano M0-T11 e M1-T07
// ("sem retry de provedor", D6: o fallback e do combo, nao do app).
//
// ATENCAO (escrito sem Rust no WSL2, nao compilado; conferir no `cargo build` do Windows):
//  - `reqwest::Response::chunk()` -> `Result<Option<Bytes>, reqwest::Error>` (nao exige `futures`).
//    Por isso a versao simples: coleta tudo num `StreamReport` em vez de devolver `impl Stream`.
//    O first token e medido no instante em que o 1o delta com texto chega, que e o que importa.
//  - `ClientBuilder::timeout` cobre a requisicao INTEIRA (inclusive o corpo em streaming).
//  - Achado do curl real (docs/spikes.md): o 9router exige `Authorization: Bearer <token>`.
//    A chave vem de `GARAGEM_LLM_API_KEY` ou, como fallback, `ANTHROPIC_AUTH_TOKEN`. Nunca logar.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{info, warn};

const DEFAULT_BASE_URL: &str = "http://localhost:20128";
// Combo padrao do dia a dia (CLAUDE.md, "Combos 9router por tarefa"). A decisao final e a D6.
const DEFAULT_COMBO: &str = "claude-tudao";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
// Timeout so da conexao: "connection refused" volta na hora; o que este limite cobre e host
// sumido/firewall derrubando pacote (sem resposta nenhuma).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Porta propositalmente errada para o teste "gateway indisponivel" (brainstorming 2.8).
pub const UNAVAILABLE_BASE_URL: &str = "http://localhost:1";

/// Erros tipados do gateway (brainstorming: taxonomia de erros). A UI recebe so o `Display`.
#[derive(Debug, Error)]
pub enum LlmGatewayError {
    /// Conexao recusada/inalcancavel: gateway fora do ar ou URL/porta errada.
    #[error("gateway inacessivel em {url}: {detail}")]
    Unreachable { url: String, detail: String },
    /// Estourou o timeout (conexao ou leitura do stream).
    #[error("timeout apos {timeout_secs} s (gateway {url})")]
    Timeout { url: String, timeout_secs: u64 },
    /// Resposta HTTP diferente de 200. 401 = token ausente/invalido.
    #[error("gateway respondeu HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    /// Linha `data:` do SSE que nao e JSON valido ou nao tem o formato esperado.
    #[error("erro de parse do SSE: {0}")]
    Parse(String),
    /// Erro enviado pelo proprio gateway dentro do stream (`data: {"error": ...}`).
    #[error("erro dentro do stream: {0}")]
    StreamError(String),
    /// Qualquer outro erro do cliente HTTP (TLS, corpo truncado, etc.).
    #[error("erro HTTP: {0}")]
    Http(String),
}

/// Resultado de uma chamada em streaming, com as metricas que o spike precisa.
#[derive(Debug, Clone, Serialize)]
pub struct StreamReport {
    /// Tempo ate o 1o delta COM texto (ms), contado desde o envio do POST. `None` se nao veio texto.
    pub first_token_ms: Option<f64>,
    /// Tempo total ate o fim do stream (`[DONE]` ou EOF), em ms.
    pub total_ms: f64,
    /// Quantidade de chunks SSE que trouxeram texto.
    pub chunk_count: u32,
    /// Texto completo concatenado.
    pub text: String,
    /// Modelo real que respondeu (campo `model` dos chunks): mostra o que o combo resolveu.
    pub model: Option<String>,
}

pub struct LlmGateway {
    base_url: String,
    combo: String,
    api_key: Option<String>,
    timeout: Duration,
}

impl LlmGateway {
    pub fn new(base_url: impl Into<String>, combo: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            combo: combo.into(),
            api_key: api_key_from_env(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// `GARAGEM_LLM_BASE_URL`, `GARAGEM_LLM_COMBO`, `GARAGEM_LLM_TIMEOUT_SECS`; senao os defaults.
    pub fn from_env() -> Self {
        let base = std::env::var("GARAGEM_LLM_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into());
        let combo = std::env::var("GARAGEM_LLM_COMBO").unwrap_or_else(|_| DEFAULT_COMBO.into());
        let mut gw = Self::new(base, combo);
        if let Some(s) = std::env::var("GARAGEM_LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            gw.timeout = Duration::from_secs(s);
        }
        gw
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn combo(&self) -> &str {
        &self.combo
    }

    /// POST `/v1/chat/completions` com `stream: true`, le o SSE incrementalmente e mede o first token.
    pub async fn stream_completion(&self, prompt: &str) -> Result<StreamReport, LlmGatewayError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let timeout_secs = self.timeout.as_secs();
        let map_err = |e: reqwest::Error| classify(e, &self.base_url, timeout_secs);

        // `no_proxy`: localhost nao deve passar por proxy do SO (autoProxy do WSL/Windows).
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(self.timeout)
            .no_proxy()
            .build()
            .map_err(|e| LlmGatewayError::Http(e.to_string()))?;

        let body = json!({
            "model": self.combo,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": true
        });

        let mut req = client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let start = Instant::now();
        let mut resp = req.send().await.map_err(map_err)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = status.as_u16(), "gateway respondeu nao-200");
            return Err(LlmGatewayError::HttpStatus {
                status: status.as_u16(),
                body: truncate(&body, 300),
            });
        }

        let mut sse = SseBuffer::default();
        let mut report = StreamReport {
            first_token_ms: None,
            total_ms: 0.0,
            chunk_count: 0,
            text: String::new(),
            model: None,
        };
        let mut done = false;

        while !done {
            let Some(bytes) = resp.chunk().await.map_err(map_err)? else {
                break; // EOF sem [DONE]: aceitamos (alguns gateways so fecham a conexao).
            };
            for payload in sse.push(&bytes) {
                if apply_payload(&payload, start, &mut report)? {
                    done = true;
                    break;
                }
            }
        }
        if !done {
            if let Some(payload) = sse.finish() {
                apply_payload(&payload, start, &mut report)?;
            }
        }

        report.total_ms = ms(start.elapsed());
        Ok(report)
    }
}

fn api_key_from_env() -> Option<String> {
    std::env::var("GARAGEM_LLM_API_KEY")
        .ok()
        .or_else(|| std::env::var("ANTHROPIC_AUTH_TOKEN").ok())
        .filter(|k| !k.is_empty())
}

fn classify(e: reqwest::Error, url: &str, timeout_secs: u64) -> LlmGatewayError {
    if e.is_timeout() {
        LlmGatewayError::Timeout { url: url.to_string(), timeout_secs }
    } else if e.is_connect() {
        LlmGatewayError::Unreachable { url: url.to_string(), detail: e.to_string() }
    } else {
        LlmGatewayError::Http(e.to_string())
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "..."
    }
}

/// Aplica um payload `data:` ao relatorio. Retorna `Ok(true)` no `[DONE]`.
fn apply_payload(
    payload: &str,
    start: Instant,
    report: &mut StreamReport,
) -> Result<bool, LlmGatewayError> {
    if payload == "[DONE]" {
        return Ok(true);
    }
    let v: Value = serde_json::from_str(payload)
        .map_err(|e| LlmGatewayError::Parse(format!("{e} em: {}", truncate(payload, 120))))?;

    if let Some(err) = v.get("error") {
        return Err(LlmGatewayError::StreamError(truncate(&err.to_string(), 300)));
    }
    if report.model.is_none() {
        report.model = v.get("model").and_then(Value::as_str).map(str::to_string);
    }
    // `delta.content` pode faltar (chunk de role, de finish_reason ou de usage): nao e erro.
    let content = v
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !content.is_empty() {
        if report.first_token_ms.is_none() {
            report.first_token_ms = Some(ms(start.elapsed()));
        }
        report.chunk_count += 1;
        report.text.push_str(content);
    }
    Ok(false)
}

/// Acumula bytes e devolve os payloads das linhas `data:` completas. Trabalha em bytes (so decodifica
/// UTF-8 por linha) porque um chunk de rede pode cortar um caractere multibyte ao meio.
#[derive(Default)]
struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            if let Some(p) = Self::data_payload(&line) {
                out.push(p);
            }
        }
        out
    }

    /// Ultima linha sem `\n` final, se houver.
    fn finish(&mut self) -> Option<String> {
        let line = std::mem::take(&mut self.buf);
        Self::data_payload(&line)
    }

    fn data_payload(line: &[u8]) -> Option<String> {
        let s = String::from_utf8_lossy(line);
        let s = s.trim_end_matches(['\r', '\n']);
        // Ignora comentarios (`:`), `event:`, `id:` e linhas vazias (separador de eventos).
        let rest = s.strip_prefix("data:")?;
        let rest = rest.trim_start();
        if rest.is_empty() { None } else { Some(rest.to_string()) }
    }
}

/// Resumo devolvido ao frontend pelo comando Tauri.
#[derive(Debug, Serialize)]
pub struct GatewayTestResult {
    pub base_url: String,
    pub combo: String,
    /// Modelo real que respondeu (o combo resolve para algum modelo da lista).
    pub model: Option<String>,
    pub first_token_ms: Option<f64>,
    pub total_ms: f64,
    pub chunk_count: u32,
    pub response: String,
}

async fn run_test(gw: LlmGateway, prompt: &str) -> Result<GatewayTestResult, String> {
    let start = Instant::now();
    match gw.stream_completion(prompt).await {
        Ok(r) => {
            info!(
                combo = gw.combo(),
                model = r.model.as_deref().unwrap_or("?"),
                first_token_ms = r.first_token_ms.unwrap_or(-1.0),
                total_ms = r.total_ms,
                chunks = r.chunk_count,
                "[llm-gateway] ok"
            );
            Ok(GatewayTestResult {
                base_url: gw.base_url().to_string(),
                combo: gw.combo().to_string(),
                model: r.model,
                first_token_ms: r.first_token_ms,
                total_ms: r.total_ms,
                chunk_count: r.chunk_count,
                response: r.text,
            })
        }
        Err(e) => {
            // Inclui quanto demorou para falhar: e a medida do teste "gateway fora do ar".
            let elapsed = ms(start.elapsed());
            warn!(error = %e, elapsed_ms = elapsed, "[llm-gateway] falhou");
            Err(format!("{e} (falhou apos {elapsed:.0} ms)"))
        }
    }
}

/// Spike M0-T11: chama o gateway configurado por env (ou defaults) e devolve as metricas.
#[tauri::command]
pub async fn test_llm_gateway(prompt: String) -> Result<GatewayTestResult, String> {
    run_test(LlmGateway::from_env(), &prompt).await
}

/// Spike M0-T11: mesma chamada apontando para uma porta sem ninguem escutando (brainstorming 2.8).
#[tauri::command]
pub async fn test_llm_gateway_unavailable(prompt: String) -> Result<GatewayTestResult, String> {
    let combo = std::env::var("GARAGEM_LLM_COMBO").unwrap_or_else(|_| DEFAULT_COMBO.into());
    run_test(LlmGateway::new(UNAVAILABLE_BASE_URL, combo), &prompt).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_junta_linhas_cortadas_entre_chunks() {
        let mut b = SseBuffer::default();
        assert!(b.push(b"data: {\"a\":").is_empty());
        let out = b.push(b"1}\n\ndata: [DONE]\n\n");
        assert_eq!(out, vec!["{\"a\":1}".to_string(), "[DONE]".to_string()]);
    }

    #[test]
    fn sse_nao_quebra_utf8_multibyte_cortado() {
        let full = "data: {\"x\":\"é\"}\n".as_bytes().to_vec();
        let cut = full.iter().position(|&b| b == 0xC3).unwrap() + 1; // meio do "é"
        let mut b = SseBuffer::default();
        assert!(b.push(&full[..cut]).is_empty());
        assert_eq!(b.push(&full[cut..]), vec!["{\"x\":\"é\"}".to_string()]);
    }

    #[test]
    fn apply_payload_extrai_texto_e_first_token() {
        let mut r = StreamReport {
            first_token_ms: None,
            total_ms: 0.0,
            chunk_count: 0,
            text: String::new(),
            model: None,
        };
        let start = Instant::now();
        // chunk de role: sem texto, nao conta
        apply_payload(r#"{"model":"m","choices":[{"delta":{"role":"assistant"}}]}"#, start, &mut r).unwrap();
        assert!(r.first_token_ms.is_none());
        apply_payload(r#"{"choices":[{"delta":{"content":"Oi"}}]}"#, start, &mut r).unwrap();
        assert!(r.first_token_ms.is_some());
        assert_eq!((r.chunk_count, r.text.as_str(), r.model.as_deref()), (1, "Oi", Some("m")));
        assert!(apply_payload("[DONE]", start, &mut r).unwrap());
    }

    #[test]
    fn apply_payload_json_invalido_e_erro_de_parse() {
        let mut r = StreamReport {
            first_token_ms: None,
            total_ms: 0.0,
            chunk_count: 0,
            text: String::new(),
            model: None,
        };
        let e = apply_payload("{nao json", Instant::now(), &mut r).unwrap_err();
        assert!(matches!(e, LlmGatewayError::Parse(_)));
    }
}
