import type { Page } from '@playwright/test';
import type {
	HistoryRecord,
	Import,
	LoadedEndpoint,
	ResponseData,
	SavedRequest,
	Section,
	SectionFileError,
	Snapshot
} from '../../src/lib/api';

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
		timeoutMs: 60_000,
		followRedirects: true,
		acceptInvalidCerts: false,
		proxy: '',
		requests: [],
		overlay: [],
		...over
	};
}

export function savedRequest(over: Partial<SavedRequest> = {}): SavedRequest {
	return {
		id: 'r1',
		name: 'List users',
		method: 'GET',
		path: '/users',
		body: '',
		headers: [],
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

export function historyRecord(over: Partial<HistoryRecord> = {}): HistoryRecord {
	return {
		id: 'h1',
		requestId: 'r1',
		at: 1_700_000_000_000,
		method: 'GET',
		url: 'https://api.acme.com/users',
		requestBody: '',
		response: {
			status: 200,
			statusText: 'OK',
			finalUrl: 'https://api.acme.com/users',
			headers: [{ name: 'content-type', value: 'application/json' }],
			isBinary: false,
			truncated: false,
			sizeBytes: 2,
			timing: { ttfbMs: 5, totalMs: 9 }
		},
		error: null,
		...over
	};
}

export function openApiImport(over: Partial<Import> = {}): Import {
	return {
		title: 'Petstore',
		version: '1.0.0',
		baseUrl: 'https://petstore.example.com',
		endpoints: [
			{ method: 'GET', path: '/pets', name: 'listPets', description: '', body: '' },
			{
				method: 'POST',
				path: '/pets',
				name: 'createPet',
				description: '',
				body: '{\n  "name": string\n}'
			}
		],
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

export interface MockUpdate {
	version: string;
	currentVersion?: string;
	/** Reject `downloadAndInstall` with this message. */
	downloadError?: string;
	/** `relaunch` fails after a successful install. */
	restartError?: string;
	/** Hold `downloadAndInstall` open so a test can drive progress. */
	deferDownload?: boolean;
	contentLength?: number;
	/**
	 * What successive `check()` calls report, one entry per call — the first
	 * replaces `version` for the first call, the rest for each call after.
	 * The last entry repeats once exhausted. Lets a test simulate a newer
	 * release appearing after one was declined.
	 */
	versions?: string[];
}

export interface MockOptions {
	sections?: Section[];
	/** Files `list_sections` reports as unreadable, alongside the good sections. */
	sectionErrors?: SectionFileError[];
	/** What the loader has already reported, i.e. `loader_cache`. */
	loaded?: LoadedEndpoint[];
	/** OpenAPI request-body schemas, keyed by endpoint id such as `POST /users`. */
	schemas?: Record<string, unknown>;
	/** OpenAPI response-body schemas, keyed the same way. */
	responseSchemas?: Record<string, unknown>;
	/** What a refresh reports instead. Defaults to `loaded` — no change. */
	refreshed?: LoadedEndpoint[];
	templates?: [string, string][];
	/** Held open so a test can drive the stream itself. See `chunk` below. */
	deferSend?: boolean;
	/** Held open so a test can observe the refresh while it is still running. */
	deferRefresh?: boolean;
	/** Milliseconds `save_section` takes, standing in for a real disk write. */
	saveLatencyMs?: number;
	/** Seeded history, newest first. */
	history?: HistoryRecord[];
	/** Response bodies for `history_body`, keyed by entry id. */
	historyBodies?: Record<string, string>;
	/** What a finished `send_request` resolves to when not deferred. */
	sendResponse?: ResponseData;
	/** Immediate rejection for `send_request` when not deferred. */
	sendError?: string;
	/** `has_secret` for any reference that has not been written this session. */
	hasSecret?: boolean;
	snapshot?: Snapshot;
	snapshotError?: string;
	/** Held open so a test can observe the picker while the snapshot is loading. */
	deferSnapshot?: boolean;
	openapi?: Import;
	parseError?: string;
	probeDocument?: unknown;
	probeError?: string;
	previewEndpoints?: LoadedEndpoint[];
	previewError?: string;
	saveError?: string;
	deleteSectionError?: string;
	deleteHistoryError?: string;
	signInError?: string;
	captureError?: string;
	/** When set, startup's updater check offers this version. */
	update?: MockUpdate;
}

/** What the page exposes back to the test, once `install` has run. */
declare global {
	interface Window {
		__FIBER_TEST__: {
			/** Commands invoked so far, oldest first. */
			calls: { cmd: string; args: Record<string, unknown> }[];
			/** The last `save_section` payload, if any. */
			lastSaved: unknown;
			/** Opens the body — the app clears the pane on this. */
			start(): void;
			/** Pushes a body chunk at the channel `send_request` was given. */
			chunk(text: string): void;
			/** Resolves the held-open `send_request`. */
			settle(data: ResponseData): void;
			/** Rejects the held-open `send_request`. */
			fail(message: string): void;
			/** Resolves the held-open `run_loader`. */
			finishRefresh(): void;
			/** Resolves the held-open `browser_snapshot`. */
			finishSnapshot(): void;
			/** Pushes a download-progress event at the updater channel. */
			updateProgress(chunkLength: number): void;
			/** Resolves the held-open `download_and_install`. */
			finishUpdate(): void;
			/** Rejects the held-open `download_and_install`. */
			failUpdate(message: string): void;
			/** Opens the command palette. The app fills this in once it mounts. */
			openPalette(): void;
			/** Surfaces the crash banner. The app fills this in once it mounts. */
			crash(message: string): void;
			/** Surfaces the crash banner as an unhandled rejection. */
			reject(message: string): void;
		};
	}
}

export async function commands(
	page: Page,
	cmd: string
): Promise<{ cmd: string; args: Record<string, unknown> }[]> {
	return page.evaluate(
		(name) => window.__FIBER_TEST__.calls.filter((call) => call.cmd === name),
		cmd
	);
}

export async function install(page: Page, options: MockOptions = {}): Promise<void> {
	await page.addInitScript(
		(opts: MockOptions) => {
			const calls: { cmd: string; args: Record<string, unknown> }[] = [];
			const callbacks = new Map<number, { fn: (payload: unknown) => void; once: boolean }>();
			let nextCallbackId = 1;

			const secrets = new Map<string, boolean>();
			let lastSaved: unknown = null;

			// The channel `send_request` was handed, and the resolver for the promise
			// it is waiting on. Both are set when the app sends.
			let channelId: number | null = null;
			let messageIndex = 0;
			let settleSend: ((data: unknown) => void) | null = null;
			let rejectSend: ((error: unknown) => void) | null = null;
			let settleRefresh: (() => void) | null = null;
			let settleSnapshot: (() => void) | null = null;
			let rejectSnapshot: ((error: unknown) => void) | null = null;

			let updateChannelId: number | null = null;
			let updateIndex = 0;
			let settleUpdate: (() => void) | null = null;
			let rejectUpdate: ((error: unknown) => void) | null = null;
			let checkCallIndex = 0;

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

			function channelFrom(value: unknown): number | null {
				if (value && typeof value === 'object' && 'id' in value) {
					const id = (value as { id?: unknown }).id;
					return typeof id === 'number' ? id : null;
				}
				return null;
			}

			function emit(id: number, index: number, message: unknown) {
				internals.runCallback(id, { index, message });
			}

			/** Same join the Rust `resolve_url` command uses. */
			function joinUrl(base: string, path: string): string {
				base = (base ?? '').trim();
				path = (path ?? '').trim();
				if (path.startsWith('http://') || path.startsWith('https://')) return path;
				if (!base) return path;
				if (!path) return base.replace(/\/+$/, '');
				return `${base.replace(/\/+$/, '')}/${path.replace(/^\/+/, '')}`;
			}

			function secretOf(reference: string): boolean {
				if (secrets.has(reference)) return secrets.get(reference) === true;
				return opts.hasSecret === true;
			}

			function handle(cmd: string, args: Record<string, unknown>): Promise<unknown> {
				switch (cmd) {
					// Startup. Anything the app asks for before a test does anything.
					case 'list_sections':
						return Promise.resolve({
							sections: opts.sections ?? [],
							errors: opts.sectionErrors ?? []
						});
					case 'history_list':
						return Promise.resolve(opts.history ?? []);
					case 'history_body':
						return Promise.resolve(opts.historyBodies?.[String(args.id)] ?? null);
					case 'history_delete':
						if (opts.deleteHistoryError) return Promise.reject(new Error(opts.deleteHistoryError));
						return Promise.resolve(null);
					case 'history_clear_request':
						if (opts.deleteHistoryError) return Promise.reject(new Error(opts.deleteHistoryError));
						return Promise.resolve(null);
					case 'history_clear_all':
						if (opts.deleteHistoryError) return Promise.reject(new Error(opts.deleteHistoryError));
						return Promise.resolve(null);
					// The quit-flush wiring listens for `flush-before-exit` and for the
					// window's own close event; both go through the event plugin. The
					// resolved number stands in for Tauri's event id.
					case 'plugin:event|listen':
						return Promise.resolve(0);
					case 'plugin:event|unlisten':
						return Promise.resolve(null);
					case 'flush_complete':
						return Promise.resolve(null);
					case 'sections_path':
						return Promise.resolve('/tmp/fiber/sections');
					case 'plugin:app|version':
						return Promise.resolve('0.0.0-test');
					case 'plugin:resources|close':
						return Promise.resolve(null);
				case 'plugin:updater|check': {
					if (!opts.update) return Promise.resolve(null);
					const versions = opts.update.versions;
					const version = versions?.length
						? versions[Math.min(checkCallIndex, versions.length - 1)]
						: opts.update.version;
					checkCallIndex++;
					return Promise.resolve({
						rid: 1,
						currentVersion: opts.update.currentVersion ?? '0.0.0-test',
						version,
						date: '2026-01-01',
						body: '',
						rawJson: {}
					});
				}
					case 'plugin:updater|download_and_install': {
						updateChannelId = channelFrom(args.onEvent);
						updateIndex = 0;
						const length = opts.update?.contentLength ?? 1000;
						const emitUpdate = (message: unknown) => {
							if (updateChannelId === null) return;
							emit(updateChannelId, updateIndex++, message);
						};
						if (opts.update?.downloadError && !opts.update.deferDownload) {
							return Promise.reject(new Error(opts.update.downloadError));
						}
						if (!opts.update?.deferDownload) {
							emitUpdate({ event: 'Started', data: { contentLength: length } });
							emitUpdate({ event: 'Progress', data: { chunkLength: length } });
							emitUpdate({ event: 'Finished' });
							return Promise.resolve(null);
						}
						return new Promise((resolve, reject) => {
							settleUpdate = () => {
								emitUpdate({ event: 'Finished' });
								resolve(null);
							};
							rejectUpdate = reject;
							emitUpdate({
								event: 'Started',
								data: { contentLength: opts.update?.contentLength ?? 1000 }
							});
						});
					}
					case 'plugin:process|restart':
						if (opts.update?.restartError) {
							return Promise.reject(new Error(opts.update.restartError));
						}
						return Promise.resolve(null);

					case 'loader_cache':
						return Promise.resolve({ loadedAt: 1, endpoints: opts.loaded ?? [] });
					case 'loader_schema':
						return Promise.resolve({
							request: opts.schemas?.[String(args.endpointId)] ?? null,
							response: opts.responseSchemas?.[String(args.endpointId)] ?? null
						});

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
					case 'loader_probe':
						if (opts.probeError) return Promise.reject(new Error(opts.probeError));
						return Promise.resolve(opts.probeDocument ?? { ok: true });
					case 'loader_preview':
						if (opts.previewError) return Promise.reject(new Error(opts.previewError));
						return Promise.resolve(opts.previewEndpoints ?? []);

					case 'parse_openapi':
						if (opts.parseError) return Promise.reject(new Error(opts.parseError));
						return Promise.resolve(
							opts.openapi ?? {
								title: 'Petstore',
								version: '1.0.0',
								baseUrl: 'https://petstore.example.com',
								endpoints: [
									{ method: 'GET', path: '/pets', name: 'listPets', description: '', body: '' },
									{
										method: 'POST',
										path: '/pets',
										name: 'createPet',
										description: '',
										body: '{\n  "name": string\n}'
									}
								]
							}
						);

					case 'browser_snapshot': {
						if (opts.snapshotError && !opts.deferSnapshot) {
							return Promise.reject(new Error(opts.snapshotError));
						}
						const snap = opts.snapshot ?? {
							localStorage: [{ key: 'auth0.token', value: 'ey.header.payload', path: '' }],
							cookies: [
								{ name: 'session', value: 'abc123', domain: 'acme.com', httpOnly: true }
							],
							indexedDb: []
						};
						if (!opts.deferSnapshot) return Promise.resolve(snap);
						return new Promise((resolve, reject) => {
							settleSnapshot = () => resolve(snap);
							rejectSnapshot = reject;
						});
					}
					case 'browser_sign_in':
						if (opts.signInError) return Promise.reject(new Error(opts.signInError));
						return Promise.resolve(null);
					case 'browser_capture': {
						if (opts.captureError) return Promise.reject(new Error(opts.captureError));
						const sectionId = String(args.sectionId ?? '');
						secrets.set(`${sectionId}:auth`, true);
						return Promise.resolve(null);
					}
					case 'browser_close':
						return Promise.resolve(null);

					case 'has_secret':
						return Promise.resolve(secretOf(String(args.reference ?? '')));
					case 'set_secret':
						secrets.set(String(args.reference ?? ''), true);
						return Promise.resolve(null);
					case 'delete_secret':
						secrets.set(String(args.reference ?? ''), false);
						return Promise.resolve(null);
					case 'forget_token':
						return Promise.resolve(null);

					case 'save_section':
						lastSaved = args.section;
						if (opts.saveError) return Promise.reject(new Error(opts.saveError));
						if (opts.saveLatencyMs) {
							return new Promise((resolve) => setTimeout(resolve, opts.saveLatencyMs));
						}
						return Promise.resolve(null);
					case 'delete_section':
						if (opts.deleteSectionError) {
							return Promise.reject(new Error(opts.deleteSectionError));
						}
						return Promise.resolve(null);
					case 'resolve_url':
						return Promise.resolve(joinUrl(String(args.base ?? ''), String(args.path ?? '')));

					case 'send_request': {
						channelId = channelFrom(args.onBody);
						messageIndex = 0;
						if (opts.sendError && !opts.deferSend) {
							return Promise.reject(new Error(opts.sendError));
						}
						if (!opts.deferSend) {
							return Promise.resolve(
								opts.sendResponse ?? {
									status: 200,
									statusText: 'OK',
									finalUrl: 'https://api.acme.com/users',
									headers: [{ name: 'content-type', value: 'application/json' }],
									isBinary: false,
									truncated: false,
									sizeBytes: 2,
									timing: { ttfbMs: 5, totalMs: 9 },
									body: '{}'
								}
							);
						}
						// Held open, so the test decides when the body arrives and when
						// the request finishes.
						return new Promise((resolve, reject) => {
							settleSend = resolve;
							rejectSend = reject;
						});
					}
					case 'cancel_request':
						if (rejectSend) {
							rejectSend(new Error('cancelled'));
							rejectSend = null;
							settleSend = null;
						}
						return Promise.resolve(true);

					// Anything unmocked is a real gap rather than something to paper
					// over: a silent `undefined` shows up much later as a confusing
					// render, so fail where the cause is.
					default:
						return Promise.reject(new Error(`no mock for Tauri command: ${cmd}`));
				}
			}

			// Cast rather than declared: @tauri-apps/api owns this global's type, and
			// a second declaration here would collide with it.
			(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = internals;
			(window as unknown as Record<string, unknown>).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
				unregisterListener() {}
			};
			window.__FIBER_TEST__ = {
				calls,
				get lastSaved() {
					return lastSaved;
				},
				start() {
					if (channelId === null) throw new Error('nothing has called send_request yet');
					emit(channelId, messageIndex++, { event: 'start' });
				},
				chunk(text: string) {
					if (channelId === null) throw new Error('nothing has called send_request yet');
					emit(channelId, messageIndex++, { event: 'chunk', data: { text } });
				},
				settle(data: unknown) {
					settleSend?.(data);
					settleSend = null;
					rejectSend = null;
				},
				fail(message: string) {
					rejectSend?.(new Error(message));
					rejectSend = null;
					settleSend = null;
				},
				finishRefresh() {
					settleRefresh?.();
					settleRefresh = null;
				},
				finishSnapshot() {
					if (opts.snapshotError) {
						rejectSnapshot?.(new Error(opts.snapshotError));
					} else {
						settleSnapshot?.();
					}
					settleSnapshot = null;
					rejectSnapshot = null;
				},
				updateProgress(chunkLength: number) {
					if (updateChannelId === null) throw new Error('nothing has called download_and_install yet');
					emit(updateChannelId, updateIndex++, {
						event: 'Progress',
						data: { chunkLength }
					});
				},
				finishUpdate() {
					settleUpdate?.();
					settleUpdate = null;
					rejectUpdate = null;
				},
				failUpdate(message: string) {
					rejectUpdate?.(new Error(message));
					rejectUpdate = null;
					settleUpdate = null;
				},
				openPalette() {
					throw new Error('app has not registered openPalette');
				},
				crash(_message: string) {
					throw new Error('app has not registered crash');
				},
				reject(_message: string) {
					throw new Error('app has not registered reject');
				}
			};
		},
		{ templates: TEMPLATES, ...options }
	);
}
