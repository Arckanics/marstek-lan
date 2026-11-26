// Contrôles de la fenêtre sans décorations (barre de titre maison, feux macOS). Hors Tauri (aperçu navigateur), tout est
// sans effet. L'API fenêtre est importée paresseusement pour ne pas charger Tauri dans le mock.
const inTauri = "__TAURI_INTERNALS__" in window;

type Win = { close(): Promise<void>; minimize(): Promise<void>; toggleMaximize(): Promise<void>; startResizeDragging(d: string): Promise<void> };
let win: Promise<Win | null> | undefined;
const w = () => (win ??= inTauri ? import("@tauri-apps/api/window").then((m) => m.getCurrentWindow() as unknown as Win) : Promise.resolve(null));

export const closeWin = () => w().then((x) => x?.close());
export const minimizeWin = () => w().then((x) => x?.minimize());
export const toggleMaximize = () => w().then((x) => x?.toggleMaximize());

/** Démarre un redimensionnement natif depuis un bord/coin (la fenêtre sans décorations n'a plus de poignées système). */
export const startResize = (dir: string) => (e: MouseEvent) => {
  if (e.button !== 0) return;
  e.preventDefault();
  void w().then((x) => x?.startResizeDragging(dir));
};
