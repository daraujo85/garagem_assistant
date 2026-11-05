// "Cerebro" do assistente. Declarado em main.rs como `brain` (ver nota la).
pub mod dispatcher;
// Spike de IPC (M0-T10): emit x Channel. Benchmark sintetico, descartavel apos a decisao.
pub mod ipc_bench;
// Spike do gateway (M0-T11): cliente HTTP/SSE do 9router. Conectividade e latencia, sem roteamento.
pub mod llm_gateway;
pub mod mcp_discovery;
pub mod prompt_engine;
