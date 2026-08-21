import type { Page } from '@playwright/test';
import type { LoadedEndpoint, ResponseData, Section } from '../../src/lib/api';

/**
 * A fake Tauri backend, installed before the app's own scripts run.
 *
 * The app reaches Rust through `window.__TAURI_INTERNALS__`, so standing that
 * object up is the whole trick — no Rust, no packaged binary, and the tests run
 * on any platform in a normal browser.
 *
 * The trade is that these tests prove the UI works against the *contract*, not
 * against the real backend. A mock that drifts from the Rust side still passes,
 * which is what the (much slower) real-binary tests would be for. Keeping the
 * fixtures typed against `api.ts` is what stops the drift being silent.
 */

/** A section with everything defaulted, so a test only states what it cares about. */
export function section(over: Partial<Section> = {}): Section {
	return {
		id: 'sec-1',
		name: 'Acme',
		baseUrl: 'https://api.acme.com',
		collapsed: false,
		order: 0,
		auth: { kind: 'none' },
		loader: null,
		mcp: { enabled: true, allowWrites: false },
		requests: [],
		overlay: [],
		...over
	};
}

export function response(over: Partial<ResponseData> = {}): ResponseData {
	return {
		status: 200,
		statusText: 'OK',
		finalUrl: 'https://api.acme.com/users',
		headers: [{ name: 'content-type', value: 'application/json' }],
		isBinary: false,
		truncated: false,
		sizeBytes: 2,
		timing: { ttfbMs: 5, totalMs: 9 },
		body: '{}',
		...over
	};
}

/** What `loader_templates` offers. Shared so a test can name one exactly. */
export const TEMPLATES: [string, string][] = [
	[
		'OpenAPI',
		'.paths | to_entries | map(.key as $path | .value | to_entries | map({method: .key, path: $path, name: $path})) | flatten'
	],
	['Array of routes', '.routes | map({method: .verb, path: .url, name: .handler})']
];

export interface MockOptions {
	sections?: Section[];
	/** What the loader has already reported, i.e. `loader_cache`. */
	loaded?: LoadedEndpoint[];
	/** What a refresh reports instead. Defaults to `loaded` — no change. */
	refreshed?: LoadedEndpoint[];
	templates?: [string, string][];
	/** Held open so a test can drive the stream itself. See `chunk` below. */
	deferSend?: boolean;
	/** Held open so a test can observe the refresh while it is still running. */
	deferRefresh?: boolean;
	/** Milliseconds `save_section` takes, standing in for a real disk write. */
	saveLatencyMs?: number;
}

/** What the page exposes back to the test, once `install` has run. */
declare global {
	interface Window {
		__FIBER_TEST__: {
			/** Commands invoked so far, oldest first. */
			calls: { cmd: string; args: Record<string, unknown> }[];
			/** Opens the body — the app clears the pane on this. */
			start(): void;
			/** Pushes a body chunk at the channel `send_request` was given. */
			chunk(text: string): void;
			/** Resolves the held-open `send_request`. */
			settle(data: ResponseData): void;
			/** Resolves the held-open `run_loader`. */
			finishRefresh(): void;
		};
	}
}

export async function install(page: Page, options: MockOptions = {}): Promise<void> {
	await page.addInitScript((opts: MockOptions) => {
		const calls: { cmd: string; args: Record<string, unknown> }[] = [];
		const callbacks = new Map<number, { fn: (payload: unknown) => void; once: boolean }>();
		let nextCallbackId = 1;

		// The channel `send_request` was handed, and the resolver for the promise
		// it is waiting on. Both are set when the app sends.
		let channelId: number | null = null;
		let messageIndex = 0;
		let settleSend: ((data: unknown) => void) | null = null;
		let settleRefresh: (() => void) | null = null;

		const internals = {
			callbacks,
            transformCallback(fn: (payload: unknown) => void, once = false) {
				const id = nextCallbackId++;
				callbacks.set(id, { fn, once });
				return id;
			},
			unregisterCallback(id: number) {
				callbacks.delete(id);
			},
			runCallback(id: number, payload: unknown) {
				const entry = callbacks.get(id);
				if (!entry) return;
				entry.fn(payload);
				if (entry.once) callbacks.delete(id);
			},
			convertFileSrc: (path: string) => path,
			metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
			invoke(cmd: string, args: Record<string, unknown> = {}) {
				calls.push({ cmd, args });
				return handle(cmd, args);
			}
		};

		function handle(cmd: string, args: Record<string, unknown>): Promise<unknown> {
			switch (cmd) {
				// Startup. Anything the app asks for before a test does anything.
				case 'list_sections':
					return Promise.resolve(opts.sections ?? []);
				case 'history_list':
					return Promise.resolve([]);
				case 'sections_path':
					return Promise.resolve('/tmp/fiber/sections');
				case 'plugin:app|version':
					return Promise.resolve('0.0.0-test');
				// No update, ever. The toast is not what these tests are about.
				case 'plugin:updater|check':
					return Promise.resolve(null);

				case 'loader_cache':
					return Promise.resolve({ loadedAt: 1, endpoints: opts.loaded ?? [] });

				case 'run_loader': {
					const endpoints = opts.refreshed ?? opts.loaded ?? [];
					const before = (opts.loaded ?? []).map((e) => `${e.method} ${e.path}`);
					const after = endpoints.map((e) => `${e.method} ${e.path}`);
					const run = {
						loadedAt: 2,
						endpoints,
						added: after.filter((key) => !before.includes(key)),
						removed: before.filter((key) => !after.includes(key)),
						pages: 1
					};
					if (!opts.deferRefresh) return Promise.resolve(run);
					// Held open, so a test can look at the app mid-refresh.
					return new Promise((resolve) => {
						settleRefresh = () => resolve(run);
					});
				}
				case 'loader_templates':
					return Promise.resolve(opts.templates ?? []);
				case 'default_loader':
					return Promise.resolve({
						enabled: true,
						url: '/openapi.json',
						method: 'GET',
						query: '.paths',
						next: '',
						ttlSeconds: 0
					});

				case 'browser_snapshot':
					return Promise.resolve({
						localStorage: [{ key: 'auth0.token', value: 'ey.header.payload', path: '' }],
						cookies: [
							{ name: 'session', value: 'abc123', domain: 'acme.com', httpOnly: true }
						],
						indexedDb: []
					});

				case 'has_secret':
					return Promise.resolve(false);
				case 'save_section':
					if (opts.saveLatencyMs) {
						return new Promise((resolve) => setTimeout(resolve, opts.saveLatencyMs));
					}
					return Promise.resolve(null);
				case 'delete_section':
					return Promise.resolve(null);
				case 'resolve_url':
					return Promise.resolve('https://api.acme.com/users');

				case 'send_request': {
					const channel = args.onBody as { id?: number } | undefined;
					channelId = channel?.id ?? null;
					messageIndex = 0;
					if (!opts.deferSend) {
						return Promise.resolve({
							status: 200,
							statusText: 'OK',
							finalUrl: 'https://api.acme.com/users',
							headers: [{ name: 'content-type', value: 'application/json' }],
							isBinary: false,
							truncated: false,
							sizeBytes: 2,
							timing: { ttfbMs: 5, totalMs: 9 },
							body: '{}'
						});
					}
					// Held open, so the test decides when the body arrives and when
					// the request finishes.
					return new Promise((resolve) => {
						settleSend = resolve;
					});
				}

				// Anything unmocked is a real gap rather than something to paper
				// over: a silent `undefined` shows up much later as a confusing
				// render, so fail where the cause is.
				default:
					return Promise.reject(new Error(`no mock for Tauri command: ${cmd}`));
			}
		}

		// Messages carry an index because the Channel replays them in order; the
		// app never sees one out of sequence.
		function emit(message: unknown) {
			if (channelId === null) throw new Error('nothing has called send_request yet');
			internals.runCallback(channelId, { index: messageIndex++, message });
		}

		// Cast rather than declared: @tauri-apps/api owns this global's type, and
		// a second declaration here would collide with it.
		(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = internals;
		window.__FIBER_TEST__ = {
			calls,
			start() {
				emit({ event: 'start' });
			},
			chunk(text: string) {
				emit({ event: 'chunk', data: { text } });
			},
			settle(data: unknown) {
				settleSend?.(data);
				settleSend = null;
			},
			finishRefresh() {
				settleRefresh?.();
				settleRefresh = null;
			}
		};
	}, { templates: TEMPLATES, ...options });
}
