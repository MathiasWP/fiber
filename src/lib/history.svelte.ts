import type { ResponseData } from './api';

/** Requests that aren't saved to a section all share this bucket. */
export const SCRATCH_ID = 'scratch';

export interface HistoryEntry {
	id: string;
	/** The saved request this belongs to, or `SCRATCH_ID`. */
	requestId: string;
	at: number;
	method: string;
	url: string;
	requestBody: string;
	/** Exactly one of these is set once the entry settles. */
	response?: ResponseData;
	error?: string;
	pending: boolean;
}

/**
 * Session history, bucketed per request.
 *
 * Each request remembers which of its own responses you were last looking at,
 * and never shows another request's. Step 3 moves this to SQLite and spills
 * bodies over ~256KB to disk; the shape the UI sees stays the same.
 */
class History {
	entries = $state<HistoryEntry[]>([]);
	/** requestId → entryId the user last looked at. */
	#selected = $state<Record<string, string>>({});

	/** Newest first. */
	forRequest(requestId: string): HistoryEntry[] {
		return this.entries.filter((entry) => entry.requestId === requestId);
	}

	/** The entry on screen for a request — an explicit pick, else its newest. */
	selectedFor(requestId: string): HistoryEntry | undefined {
		const mine = this.forRequest(requestId);
		const picked = this.#selected[requestId];
		return mine.find((entry) => entry.id === picked) ?? mine[0];
	}

	select(requestId: string, entryId: string): void {
		this.#selected[requestId] = entryId;
	}

	start(entry: Omit<HistoryEntry, 'pending'>): void {
		this.entries.unshift({ ...entry, pending: true });
		this.select(entry.requestId, entry.id);
	}

	settle(id: string, result: { response?: ResponseData; error?: string }): void {
		const entry = this.entries.find((candidate) => candidate.id === id);
		if (!entry) return;
		entry.pending = false;
		entry.response = result.response;
		entry.error = result.error;
	}

	remove(id: string): void {
		this.entries = this.entries.filter((entry) => entry.id !== id);
	}

	clearFor(requestId: string): void {
		this.entries = this.entries.filter((entry) => entry.requestId !== requestId);
		delete this.#selected[requestId];
	}

	clear(): void {
		this.entries = [];
		this.#selected = {};
	}
}

export const history = new History();
