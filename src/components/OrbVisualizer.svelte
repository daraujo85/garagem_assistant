<script>
  // Placeholder em CSS puro. O render WebGL/shader e do M1 (ver M0-T09 para o spike de janela).
  let { state = "Idle" } = $props();

  const colors = {
    Idle: "#8a8a99",
    Listening: "#2ecc71",
    Thinking: "#f1c40f",
    Speaking: "#3498db",
    Error: "#e74c3c",
  };

  let color = $derived(colors[state] ?? "#555566");
</script>

<div class="orb" style:background={color} style:box-shadow={`0 0 40px ${color}`}></div>

<style>
  .orb {
    width: 160px;
    height: 160px;
    border-radius: 50%;
    transition: background 0.3s ease, box-shadow 0.3s ease;
  }
</style>
