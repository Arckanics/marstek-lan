import { mount } from "svelte";
import "uplot/dist/uPlot.min.css";
import "./app.css";
import App from "./App.svelte";

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) (await import("./lib/mock")).installMock();

mount(App, { target: document.getElementById("app")! });
