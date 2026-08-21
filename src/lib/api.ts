import { Channel, invoke } from '@tauri-apps/api/core';

export interface Header {
	name: string;
	value: string;
}

export interface RequestSpec {
	/** Also the cancellation handle and the history entry's primary key. */
	id: string;
	/** The saved request this belongs to, so history buckets it correctly. */
	requestId: string;
	/** The section this came from — its auth config is applied on send. */
	sectionId?: string | null;
	method: string;
	url: string;
	headers: Header[];
	body?: string | null;
	timeoutMs?: number | null;
	followRedirects?: boolean;
	acceptInvalidCerts?: boolean;
}

export interface Timing {
	ttfbMs: number;
	totalMs: number;
}

/** Everything about a response except the body, which is fetched separately. */
export interface ResponseMeta {
	status: number;
	statusText: string;
	finalUrl: string;
	headers: Header[];
	isBinary: boolean;
	truncated: boolean;
	sizeBytes: number;
	timing: Timing;
}

export interface ResponseData extends ResponseMeta {
	/** UTF-8 text, or base64 when `isBinary`. */
	body: string;
}

/** A stored history entry. Bodies come from `historyBody`. */
export interface HistoryRecord {
	id: string;
	requestId: string;
	at: number;
	method: string;
	url: string;
	requestBody: string;
	response: ResponseMeta | null;
	error: string | null;
}

export function historyList(): Promise<HistoryRecord[]> {
	return invoke<HistoryRecord[]>('history_list');
}

export function historyBody(id: string): Promise<string | null> {
	return invoke<string | null>('history_body', { id });
}

export function historyDelete(id: string): Promise<void> {
	return invoke<void>('history_delete', { id });
}

export function historyClearRequest(requestId: string): Promise<void> {
	return invoke<void>('history_clear_request', { requestId });
}

export function historyClearAll(): Promise<void> {
	return invoke<void>('history_clear_all');
}

/** A request saved inside a section. `path` is relative to the section's base URL. */
export interface SavedRequest {
	id: string;
	name: string;
	method: string;
	path: string;
	body: string;
	headers: Header[];
}

/**
 * How a section obtains credentials. Values never live here — `secretRef` names
 * an entry in the OS keychain, so a section file is safe to share.
 */
export type AuthConfig =
	| { kind: 'none' }
	| { kind: 'bearer'; secretRef: string }
	| {
			kind: 'login';
			method: string;
			/** Absolute, or relative to the section's base URL. */
			url: string;
			/** Where the token is in the response, e.g. `$.data.access_token`. */
			tokenPath: string;
			header: string;
			prefix: string;
			/** 0 means "cache until a 401 says otherwise". */
			ttlSeconds: number;
			/** Keychain reference for the login request body. */
			secretRef: string;
	  }
	| {
			kind: 'browser';
			/** The page to open for signing in. */
			loginUrl: string;
			capture: CaptureKind;
			/** localStorage key, or cookie name. */
			captureKey: string;
			/** Dotted path into the stored JSON. Empty means the raw value. */
			capturePath: string;
			header: string;
			prefix: string;
			ttlSeconds: number;
			secretRef: string;
	  };

export type AuthKind = AuthConfig['kind'];
export type CaptureKind = 'localStorage' | 'cookie' | 'indexedDb';

export interface StorageEntry {
	key: string;
	value: string;
}

/** One IndexedDB record. Identified by `database/store/key`. */
export interface IndexedEntry {
	database: string;
	store: string;
	key: string;
	value: string;
}

export interface CookieEntry {
	name: string;
	value: string;
	domain: string;
	/** True when the page's own JavaScript could not have read this. */
	httpOnly: boolean;
}

/** Everything the signed-in browser session holds. */
export interface Snapshot {
	localStorage: StorageEntry[];
	cookies: CookieEntry[];
	indexedDb: IndexedEntry[];
}

/** Opens a real browser window at the section's login page. */
export function browserSignIn(sectionId: string): Promise<void> {
	return invoke<void>('browser_sign_in', { sectionId });
}

export function browserSnapshot(sectionId: string): Promise<Snapshot> {
	return invoke<Snapshot>('browser_snapshot', { sectionId });
}

/** Applies the section's saved capture rule and stores what it finds. */
export function browserCapture(sectionId: string): Promise<void> {
	return invoke<void>('browser_capture', { sectionId });
}

export function browserClose(sectionId: string): Promise<void> {
	return invoke<void>('browser_close', { sectionId });
}

/**
 * How a section discovers its endpoints: fetch a manifest, map it with a jq
 * filter. A filter is a transformation, not a program — it can't reach anything
 * but the document, which is what lets the editor preview it as you type.
 */
export interface LoaderConfig {
	enabled: boolean;
	/** Where the manifest lives, relative to the section's base URL. */
	url: string;
	method: string;
	/** jq filter producing `{method, path, name?}` objects. */
	query: string;
	/** jq filter yielding the next page's URL, or null when done. */
	next: string;
	/** 0 means "only when asked". */
	ttlSeconds: number;
}

/** One endpoint as reported by a loader. Never the source of truth for user data. */
export interface LoadedEndpoint {
	method: string;
	path: string;
	name: string;
	description: string;
	/** A JSON body to start from, when the manifest was an OpenAPI document. */
	body: string;
}

export interface LoaderCache {
	loadedAt: number;
	endpoints: LoadedEndpoint[];
}

export interface LoaderRun extends LoaderCache {
	added: string[];
	removed: string[];
	pages: number;
}

/**
 * What the MCP server may do with a section. Off by default — a tool that can
 * send authenticated requests is a confused deputy, so exposure is a decision
 * made per collection.
 */
export interface McpAccess {
	/** Whether agents can see this collection at all. */
	enabled: boolean;
	/** Whether they may use anything but GET, HEAD and OPTIONS. */
	allowWrites: boolean;
}

/** A group of requests sharing a base URL. */
export interface Section {
	id: string;
	name: string;
	baseUrl: string;
	collapsed: boolean;
	/** Where this sits in the sidebar; set by dragging. */
	order: number;
	auth: AuthConfig;
	loader?: LoaderConfig | null;
	mcp: McpAccess;
	/** Hand-written requests. */
	requests: SavedRequest[];
	/** User data for *loaded* endpoints, keyed by `id` = `"GET /path"`. */
	overlay: SavedRequest[];
}

/**
 * The reserved section holding requests that belong to no collection.
 *
 * It's an ordinary section on disk — same persistence, selection, history and
 * editing — but it has no base URL and the sidebar renders its requests at the
 * top rather than under a header, so it reads as "loose" rather than as a
 * collection you have to name.
 */
export const LOOSE_SECTION_ID = 'loose';

/** One `name=value` pair from a URL's query string. */
export interface QueryParam {
	name: string;
	value: string;
}

/**
 * Splits a path into everything before the first `?` and the query after it.
 *
 * Query parameters are not stored anywhere: they are part of the path, and the
 * params table is a second view onto the same string the URL bar shows. That
 * keeps the two in step for free, needs no new field in the section files, and
 * means a URL pasted in with its query already attached simply appears in the
 * table.
 */
export function splitQuery(path: string): { base: string; query: string } {
	const at = path.indexOf('?');
	if (at < 0) return { base: path, query: '' };
	return { base: path.slice(0, at), query: path.slice(at + 1) };
}

export function parseQuery(path: string): QueryParam[] {
	const { query } = splitQuery(path);
	if (!query) return [];
	// URLSearchParams keeps duplicates and order, both of which are meaningful.
	return [...new URLSearchParams(query)].map(([name, value]) => ({ name, value }));
}

/**
 * Puts `params` back onto `path`, replacing whatever query it had.
 *
 * Pairs with a blank name are dropped — the table always carries one empty row
 * to type into, and that row is not a parameter until it is named.
 */
export function withQuery(path: string, params: QueryParam[]): string {
	const { base } = splitQuery(path);
	const search = new URLSearchParams();
	for (const { name, value } of params) {
		if (name.trim()) search.append(name, value);
	}
	const query = search.toString();
	return query ? `${base}?${query}` : base;
}

/** The stable identity that ties saved bodies and history to a loaded endpoint. */
export function endpointKey(method: string, path: string): string {
	return `${method.trim().toUpperCase()} ${path.trim()}`;
}

/** Runs the loader. Its `fetch` uses the section's base URL and auth. */
export function runLoader(sectionId: string): Promise<LoaderRun> {
	return invoke<LoaderRun>('run_loader', { sectionId });
}

/** The last successful run, from disk — available immediately and offline. */
export function loaderCache(sectionId: string): Promise<LoaderCache> {
	return invoke<LoaderCache>('loader_cache', { sectionId });
}

/** Fetches the manifest untouched, for the filter editor to preview against. */
export function loaderProbe(sectionId: string): Promise<unknown> {
	return invoke<unknown>('loader_probe', { sectionId });
}

/**
 * Applies a filter to a document already in hand. Pure — no network, no
 * section — so it's safe to call on every keystroke.
 */
export function loaderPreview(document: unknown, query: string): Promise<LoadedEndpoint[]> {
	return invoke<LoadedEndpoint[]>('loader_preview', { document, query });
}

export function defaultLoader(): Promise<LoaderConfig> {
	return invoke<LoaderConfig>('default_loader');
}

export interface ImportedEndpoint {
	method: string;
	path: string;
	name: string;
	description: string;
	/** A JSON body to start from, or empty when the operation takes none. */
	body: string;
}

export interface Import {
	title: string;
	version: string;
	baseUrl: string;
	endpoints: ImportedEndpoint[];
}

/**
 * Parses an OpenAPI or Swagger document, JSON or YAML. Pure — nothing is
 * written until the user confirms, and the result becomes ordinary requests
 * rather than loader output, so it keeps working offline.
 */
export function parseOpenApi(text: string): Promise<Import> {
	return invoke<Import>('parse_openapi', { text });
}

/** Worked filters for the manifest shapes people actually hit. */
export function loaderTemplates(): Promise<[string, string][]> {
	return invoke<[string, string][]>('loader_templates');
}

/** Write-only by design: there is no command to read a secret back out. */
export function setSecret(reference: string, value: string): Promise<void> {
	return invoke<void>('set_secret', { reference, value });
}

export function hasSecret(reference: string): Promise<boolean> {
	return invoke<boolean>('has_secret', { reference });
}

export function deleteSecret(reference: string): Promise<void> {
	return invoke<void>('delete_secret', { reference });
}

/** Forces the next send for this section to log in again. */
export function forgetToken(sectionId: string): Promise<void> {
	return invoke<void>('forget_token', { sectionId });
}

/** A section file that could not be read. The file itself is left untouched. */
export interface SectionFileError {
	/** The file name, e.g. `books.toml`. */
	file: string;
	/** What the parser objected to. */
	message: string;
}

/**
 * Every section that could be read, and a report on every file that couldn't.
 *
 * The split matters: one corrupt TOML file must not take the rest of the
 * collections down with it, but skipping it silently would look like data
 * loss — the file still exists on disk, and the user is the one who can fix it.
 */
export interface SectionList {
	sections: Section[];
	errors: SectionFileError[];
}

export function listSections(): Promise<SectionList> {
	return invoke<SectionList>('list_sections');
}

export function saveSection(section: Section): Promise<void> {
	return invoke<void>('save_section', { section });
}

export function deleteSection(id: string): Promise<void> {
	return invoke<void>('delete_section', { id });
}

/**
 * Resolution lives in Rust so the app and the MCP server can never disagree
 * about where a request actually goes. The UI previews what this returns.
 */
export function resolveUrl(base: string, path: string): Promise<string> {
	return invoke<string>('resolve_url', { base, path });
}

export function sectionsPath(): Promise<string> {
	return invoke<string>('sections_path');
}

export const METHODS = [
	'GET',
	'POST',
	'PUT',
	'PATCH',
	'DELETE',
	'HEAD',
	'OPTIONS'
] as const;

export type Method = (typeof METHODS)[number];

/** Requests are sent from Rust — see `src-tauri/src/http.rs` for why. */
/**
 * A response body arriving in pieces.
 *
 * `start` means a body is beginning — clear whatever is shown. It arrives once
 * per attempt, so a 401 that is refreshed and retried sends a second one and
 * the retry's body replaces the first rather than continuing it.
 */
export type BodyEvent = { event: 'start' } | { event: 'chunk'; data: { text: string } };

/**
 * Sends, streaming the body to `onBody` as it arrives.
 *
 * The resolved `ResponseData` is still the authoritative copy — it is the one
 * that knows about truncation and binary bodies — so what streams is a preview
 * of it, replaced wholesale when the request finishes. Only text streams; a
 * binary body arrives once, at the end.
 */
export function sendRequest(
	spec: RequestSpec,
	onBody?: (event: BodyEvent) => void
): Promise<ResponseData> {
	const channel = new Channel<BodyEvent>();
	if (onBody) channel.onmessage = onBody;
	return invoke<ResponseData>('send_request', { spec, onBody: channel });
}

export function cancelRequest(id: string): Promise<boolean> {
	return invoke<boolean>('cancel_request', { id });
}

export function methodColor(method: string): string {
	switch (method.toUpperCase()) {
		case 'GET':
			return 'text-ok';
		case 'POST':
			return 'text-accent';
		case 'PUT':
		case 'PATCH':
			return 'text-warn';
		case 'DELETE':
			return 'text-bad';
		default:
			return 'text-muted';
	}
}

/**
 * Tells Rust that every pending save has been written, so a user-initiated
 * quit may proceed. The other half of the `flush-before-exit` event: Rust
 * holds the exit open until this arrives, or gives up after a grace period.
 */
export function flushComplete(): Promise<void> {
	return invoke<void>('flush_complete');
}

/**
 * The form a base URL is stored in: trimmed, and without a trailing slash.
 *
 * Nothing depends on it. `join_url` on the Rust side trims both sides before
 * joining, so `https://api.example.com` and `https://api.example.com/` have
 * always produced the same request. It exists so that what is on screen after
 * you finish typing is unambiguous — one canonical form to compare against,
 * rather than two that look different and behave identically.
 */
export function normalizeBaseUrl(value: string): string {
	const trimmed = value.trim();
	// A bare scheme is someone mid-type, and its slashes are structural: naively
	// stripping them turns "https://" into "https:".
	const scheme = /^([a-z][a-z0-9+.\-]*:\/\/)(.*)$/i.exec(trimmed);
	if (scheme) return scheme[1] + scheme[2].replace(/\/+$/, '');
	return trimmed.replace(/\/+$/, '');
}

export function statusColor(status: number): string {
	if (status >= 500) return 'text-bad';
	if (status >= 400) return 'text-warn';
	if (status >= 300) return 'text-accent';
	if (status >= 200) return 'text-ok';
	return 'text-muted';
}

export function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * Above this, JSON tooling steps aside: no pretty-printing, no syntax tree, no
 * lint pass. Bodies can be up to 32 MB, and every one of those passes is a
 * synchronous walk of the whole string on the main thread — a parse, a
 * stringify and a Lezer tree over a body that size is seconds of frozen UI to
 * produce indentation nobody asked for. Character count rather than bytes:
 * close enough at this scale, and it's the unit everything here already has.
 */
export const JSON_TOOLING_LIMIT = 1.5 * 1024 * 1024;

/**
 * Pretty-prints JSON, returning the input untouched when it isn't JSON — or
 * when it's too large to parse without freezing the UI (see the limit above).
 */
export function tryFormatJson(text: string): string {
	if (text.length > JSON_TOOLING_LIMIT) return text;
	try {
		return JSON.stringify(JSON.parse(text), null, 2);
	} catch {
		return text;
	}
}
