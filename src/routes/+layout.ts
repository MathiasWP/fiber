// Tauri has no server. SPA mode, and no prerendering — load functions must be
// able to reach the Tauri APIs, which only exist at runtime in the webview.
export const ssr = false;
export const prerender = false;
