import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// UnoCSS runs through PostCSS here, not through `unocss/vite`. Its Vite plugin's
// virtual module breaks SvelteKit's client under Vite 8 — the layout component
// module fails to initialise and every navigation dies with a TDZ error.
// See postcss.config.mjs and the `@unocss` directive in src/app.css.
export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			// Tauri serves a static bundle from disk — there is no server. SPA
			// fallback rather than prerendering, so load functions can reach the
			// Tauri APIs at runtime.
			adapter: adapter({ fallback: 'index.html' })
		})
	],

	// Tauri points at a fixed port, so failing is better than silently moving.
	server: {
		port: 5173,
		strictPort: true
	},

	envPrefix: ['VITE_', 'TAURI_'],
	build: {
		target: 'safari15',
		sourcemap: !!process.env.TAURI_ENV_DEBUG
	}
});
