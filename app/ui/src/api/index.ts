import { mockBackend } from "./mockBackend";
import { tauriBackend } from "./tauriBackend";
import type { Backend } from "./types";

export * from "./types";

/** The Tauri backend inside the desktop app, the fake-data backend in a plain browser. */
export const backend: Backend = "__TAURI_INTERNALS__" in window ? tauriBackend : mockBackend;
