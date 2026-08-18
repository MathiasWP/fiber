import { invoke } from '@tauri-apps/api/core';

export interface Header {
	name: string;
	value: string;
}

export interface RequestSpec {
	/** Also the cancellation handle and the history entry's primary key. */
	id: string;
	/** The saved request this belongs to, so history buckets it correctly. */
	requestId: string;
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

/** A group of requests sharing a base URL. Auth and loaders attach here later. */
export interface Section {
	id: string;
	name: string;
	baseUrl: string;
	collapsed: boolean;
	requests: SavedRequest[];
}

export function listSections(): Promise<Section[]> {
	return invoke<Section[]>('list_sections');
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
export function sendRequest(spec: RequestSpec): Promise<ResponseData> {
	return invoke<ResponseData>('send_request', { spec });
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

/** Pretty-prints JSON, returning the input untouched when it isn't JSON. */
export function tryFormatJson(text: string): string {
	try {
		return JSON.stringify(JSON.parse(text), null, 2);
	} catch {
		return text;
	}
}
