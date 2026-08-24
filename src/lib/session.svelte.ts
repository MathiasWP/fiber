import { SCRATCH_ID } from './history.svelte';
import { collections } from './collections.svelte';
import { BODY_KINDS, type SavedRequest } from './api';

/**
 * Where you were, so the next launch starts there.
 *
 * Collections, requests and responses are already durable — TOML on disk and
 * SQLite respectively. What was not, until this existed, is everything *about
 * looking at them*: which endpoint was open, which tab of it, which folders
 * were expanded, and whatever had been typed into the scratch request. An
 * ordinary quit loses that, and an auto-update restart — which nobody asked
 * for and which can land mid-sentence — loses it without even being asked.
 *
 * Window size and position are not here: `tauri-plugin-window-state` already
 * writes those on exit, and the restart path runs through exit like any other.
 *
 * One JSON blob under one key, deliberately. These fields are only meaningful
 * together — a tab without the request it belongs to says nothing — and a
 * single read at startup beats a key per field.
 *
 * Contents are as sensitive as the collection files themselves: a scratch
 * request carries whatever headers you typed into it, and it is stored in the
 * webview's local storage as plainly as a section is stored in TOML. Secrets
 * proper still live in the keychain and are still referenced by path, never
 * copied in here.
 */
const STORAGE_KEY = 'fiber:session';

/**
 * Long enough that a burst of typing costs one write, short enough that a
 * crash a moment later still remembers the right endpoint.
 */
const SAVE_DEBOUNCE_MS = 400;

/**
 * Past this the scratch body is remembered as empty rather than not at all.
 *
 * Local storage is a few megabytes for the whole origin, shared with the theme
 * and the editor sizes. A pasted payload big enough to threaten that quota is
 * also one nobody retyped from memory, so dropping it beats a failed write
 * that loses the selection along with it.
 */
const MAX_SCRATCH_BODY = 256 * 1024;

export type RequestTab = 'params' | 'body' | 'headers';
export type ResponseTab = 'pretty' | 'raw' | 'headers';
export type SidebarTab = 'collections' | 'history' | 'mcp';

const REQUEST_TABS: RequestTab[] = ['params', 'body', 'headers'];
const RESPONSE_TABS: ResponseTab[] = ['pretty', 'raw', 'headers'];
const SIDEBAR_TABS: SidebarTab[] = ['collections', 'history', 'mcp'];

/**
 * The unsaved request you get before picking anything from the sidebar.
 * Its `path` is a full URL — there's no section to hang a base off.
 */
export function blankScratch(): SavedRequest {
	return {
		id: SCRATCH_ID,
		name: 'Scratch',
		method: 'GET',
		path: '',
		body: '{\n  "hello": "world"\n}',
		bodyKind: 'json',
		form: [],
		file: '',
		pathParams: [],
		headers: []
	};
}

interface Stored {
	requestId: string | null;
	sectionId: string | null;
	requestTab: RequestTab;
	responseTab: ResponseTab;
	sidebarTab: SidebarTab;
	openTags: Record<string, boolean>;
	scratch: SavedRequest;
}

function pick<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
	return typeof value === 'string' && (allowed as readonly string[]).includes(value)
		? (value as T)
		: fallback;
}

function text(value: unknown, fallback: string): string {
	return typeof value === 'string' ? value : fallback;
}

function list<T>(value: unknown): T[] {
	return Array.isArray(value) ? (value as T[]) : [];
}

/**
 * Reads every field of a reactive value, so an effect that calls it re-runs
 * when any of them changes.
 *
 * Reading rather than snapshotting on purpose: `$state.snapshot` would clone
 * the whole request — including a body that can be megabytes — on every
 * keystroke, only to throw the clone away when the debounce coalesces it into
 * one write. This walks the same fields for free and leaves the copying to the
 * moment something is actually written.
 */
function track(value: unknown): void {
	if (Array.isArray(value)) {
		for (const entry of value) track(entry);
	} else if (value && typeof value === 'object') {
		for (const entry of Object.values(value)) track(entry);
	}
}

class Session {
	requestTab = $state<RequestTab>('body');
	responseTab = $state<ResponseTab>('pretty');
	sidebarTab = $state<SidebarTab>('collections');
	/** Loader folders the user has opened, keyed `sectionId\0tag`. */
	openTags = $state<Record<string, boolean>>({});
	scratch = $state<SavedRequest>(blankScratch());

	/**
	 * Until the stored session has been read, there is nothing worth writing —
	 * and writing anyway would replace it with this class's defaults.
	 */
	#restored = false;
	#timer: ReturnType<typeof setTimeout> | null = null;

	/**
	 * Reads the stored session. Call once, before anything renders from it and
	 * before `persist` is ever armed — a second call would put the last launch's
	 * state back over whatever has been done since.
	 */
	restore(): void {
		if (this.#restored) return;
		this.#restored = true;

		let stored: Partial<Stored> | null = null;
		try {
			stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null');
		} catch {
			// Truncated by a crash mid-write, or written by a version that shaped
			// it differently. Either way it is a starting point, not data.
		}
		if (!stored || typeof stored !== 'object') return;

		this.requestTab = pick(stored.requestTab, REQUEST_TABS, 'body');
		this.responseTab = pick(stored.responseTab, RESPONSE_TABS, 'pretty');
		this.sidebarTab = pick(stored.sidebarTab, SIDEBAR_TABS, 'collections');

		if (stored.openTags && typeof stored.openTags === 'object') {
			this.openTags = Object.fromEntries(
				Object.entries(stored.openTags).filter(([, open]) => open === true)
			);
		}

		// Field by field, and each one checked: a blob written by an older
		// version is missing fields a newer one expects, and a blob that has
		// been edited by hand can hold anything at all. A default beats a
		// `headers` that turns out not to be an array.
		//
		// Mutated in place rather than replaced: the page reads `session.scratch`
		// once and holds on to it, so the object it holds has to be the one that
		// ends up with the restored values in it.
		const scratch = stored.scratch;
		if (scratch && typeof scratch === 'object') {
			const blank = blankScratch();
			Object.assign(this.scratch, blank, {
				method: text(scratch.method, blank.method),
				path: text(scratch.path, blank.path),
				body: text(scratch.body, ''),
				bodyKind: pick(
					scratch.bodyKind,
					BODY_KINDS.map((kind) => kind.value),
					'json'
				),
				file: text(scratch.file, ''),
				form: list(scratch.form),
				pathParams: list(scratch.pathParams),
				headers: list(scratch.headers)
			});
		}

		// The selection is set now and resolved later: sections are still being
		// read off disk, so `collections.selected` answers null until they land.
		// An id that never resolves simply leaves the scratch request showing —
		// `verify` clears it once there is something to check it against.
		if (typeof stored.requestId === 'string') collections.selectedRequestId = stored.requestId;
		if (typeof stored.sectionId === 'string') collections.selectedSectionId = stored.sectionId;
	}

	/**
	 * Forgets a restored selection whose request is gone — deleted from another
	 * window, or dropped by a loader that no longer reports it. Call once the
	 * sections are loaded.
	 */
	verify(): void {
		if (collections.selectedRequestId && !collections.selected) {
			collections.selectedRequestId = null;
			collections.selectedSectionId = null;
		}
	}

	/**
	 * Notices a change and schedules a write. Call from an effect: it reads
	 * everything it stores, so the effect re-runs whenever any of it moves.
	 */
	persist(): void {
		track(this.requestTab);
		track(this.responseTab);
		track(this.sidebarTab);
		track(this.openTags);
		track(this.scratch);
		track(collections.selectedRequestId);
		track(collections.selectedSectionId);

		if (!this.#restored) return;
		if (this.#timer !== null) clearTimeout(this.#timer);
		this.#timer = setTimeout(() => {
			this.#timer = null;
			this.#write();
		}, SAVE_DEBOUNCE_MS);
	}

	/**
	 * Writes now, debounce be damned — the quit and update-restart paths, where
	 * the timer that would have fired is about to stop existing.
	 */
	flush(): void {
		if (this.#timer !== null) clearTimeout(this.#timer);
		this.#timer = null;
		if (this.#restored) this.#write();
	}

	#write(): void {
		const scratch = $state.snapshot(this.scratch) as SavedRequest;
		if (scratch.body.length > MAX_SCRATCH_BODY) scratch.body = '';

		const stored: Stored = {
			requestId: collections.selectedRequestId,
			sectionId: collections.selectedSectionId,
			requestTab: this.requestTab,
			responseTab: this.responseTab,
			sidebarTab: this.sidebarTab,
			// Only the open ones: a closed folder is the default, and keeping the
			// `false` entries would grow the blob by every folder ever clicked.
			openTags: Object.fromEntries(
				Object.entries($state.snapshot(this.openTags)).filter(([, open]) => open)
			),
			scratch
		};

		try {
			localStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
		} catch {
			// Out of quota, or storage denied outright. Where you were is a
			// convenience; nothing here is worth interrupting anyone over.
		}
	}
}

export const session = new Session();
