// Evita abrir janela de console extra no Windows em release. NAO remover.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Modulos conforme arquitetura_garagemassistant.md secao 2. Todos stubs no M0-T01.
// A pasta e `core/` (fiel a arquitetura), mas o modulo se chama `brain` via #[path]: um
// `mod core` na raiz sombrearia o crate `core` da std e pode gerar ambiguidade de paths.
// Spike de captura (M0-T05) usado pelo shortcut.rs; spike STT (M0-T06) em audio/stt.rs; tts_piper segue stub.
#[allow(dead_code)]
mod audio;
#[allow(dead_code)]
#[path = "core/mod.rs"]
mod brain;
#[allow(dead_code)]
mod db;
// Spike PTT (M0-T03). Fica fora de `core/` (brain): e infraestrutura de entrada, nao logica do assistente.
mod shortcut;

use serde::Serialize;
use tracing::info;

/// Estado da FSM do assistente. Placeholder do M1: por ora so o comando `get_state` o expoe.
#[derive(Debug, Clone, Copy, Serialize)]
enum AppState {
    Idle,
    // M1: Listening, Thinking, Speaking, Error.
}

/// Comando de exemplo para provar o IPC Rust <-> Svelte. Retorna sempre "Idle".
#[tauri::command]
fn get_state() -> AppState {
    AppState::Idle
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn main() {
    init_tracing();
    info!("GaragemAssistant iniciando");

    tauri::Builder::default()
        // Plugin com handler Pressed/Released (M0-T03). O atalho em si e registrado no `setup`.
        .plugin(shortcut::plugin())
        // Estado do spike PTT: o timestamp do Pressed (ver shortcut.rs).
        .manage(shortcut::PttState::default())
        // Estado do spike de captura (M0-T05): AudioCapture ativo durante o hold.
        .manage(shortcut::AudioState::default())
        // Estado do spike STT (M0-T06): modelo Whisper carregado lazy na primeira transcricao.
        .manage(audio::stt::SttState::default())
        .setup(|app| {
            shortcut::init_clock();
            shortcut::register(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, audio::stt::transcribe_last_capture])
        .run(tauri::generate_context!())
        .expect("erro ao executar a aplicacao Tauri");
}
