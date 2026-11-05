import { mount } from "svelte";
import { getCurrentWindow } from "@tauri-apps/api/window";

// M0-T09: o mesmo frontend serve as duas janelas do tauri.conf.json. A janela "orb" (transparente,
// pequena) monta so o orb; a "main" monta o App com as secoes de debug dos spikes. Import dinamico
// para o CSS global de cada raiz (fundo opaco do App x transparente do orb) nao vazar para a outra.
let label = "main";
try {
  label = getCurrentWindow().label;
} catch {
  // Fora do Tauri (ex.: `npm run dev` num browser comum): cai no App normal.
}

const loader = label === "orb" ? import("./OrbWindow.svelte") : import("./App.svelte");
const app = loader.then(({ default: Root }) =>
  mount(Root, { target: document.getElementById("app") })
);

export default app;
