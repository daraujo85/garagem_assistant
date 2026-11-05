// "Cerebro" do assistente. Declarado em main.rs como `brain` (ver nota la).
pub mod dispatcher;
// Spike de IPC (M0-T10): emit x Channel. Benchmark sintetico, descartavel apos a decisao.
pub mod ipc_bench;
pub mod mcp_discovery;
pub mod prompt_engine;
