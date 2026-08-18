import {
	historyBody,
	historyClearAll,
	historyClearRequest,
	historyDelete,
	historyList,
	type HistoryRecord,
	type ResponseData,
	type ResponseMeta
} from './api';

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
	response?: ResponseMeta;
	error?: string;
	/** Fetched on demand — see `ensureBody`. */
	body?: string;
	bodyLoaded: boolean;
	pending: boolean;
}

function fromRecord(record: HistoryRecord): HistoryEntry {
	return {
		id: record.id,
		requestId: record.requestId,
		at: record.at,
		method: record.method,
		url: record.url,
		requestBody: record.requestBody,
		response: record.response ?? undefined,
		error: record.error ?? undefined,
		bodyLoaded: false,
		pending: false
	};
}

/**
 * Request history, bucketed per request and persisted in SQLite by the Rust
 * side as part of sending.
 *
 * Bodies are deliberately not loaded with the list — a few hundred entries of
 * metadata is cheap, a few hundred response bodies is not. `ensureBody` pulls
 * one in when it's about to be shown.
 */
class History {
	entries = $state<HistoryEntry[]>([]);
	error = $state<string | null>(null);
	/** requestId → entryId the user last looked at. */
	#selected = $state<Record<string, string>>({});

	async load(): Promise<void> {
		try {
			this.entries = (await historyList()).map(fromRecord);
			this.error = null;
		} catch (error) {
			this.error = String(error);
		}
	}

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

	async ensureBody(entry: HistoryEntry): Promise<void> {
		if (entry.bodyLoaded || entry.pending) return;
		// Set first so concurrent calls for the same entry don't both fetch.
		entry.bodyLoaded = true;
		try {
			entry.body = (await historyBody(entry.id)) ?? '';
		} catch (error) {
			entry.bodyLoaded = false;
			this.error = String(error);
		}
	}

	start(entry: Omit<HistoryEntry, 'pending' | 'bodyLoaded'>): void {
		this.entries.unshift({ ...entry, pending: true, bodyLoaded: false });
		this.select(entry.requestId, entry.id);
	}

	/** The response is already in hand here, so its body needs no round trip. */
	settle(id: string, result: { response?: ResponseData; error?: string }): void {
		const entry = this.entries.find((candidate) => candidate.id === id);
		if (!entry) return;

		entry.pending = false;
		entry.error = result.error;
		if (result.response) {
			const { body, ...meta } = result.response;
			entry.response = meta;
			entry.body = body;
			entry.bodyLoaded = true;
		} else {
			entry.response = undefined;
			entry.bodyLoaded = true;
		}
	}

	async remove(id: string): Promise<void> {
		this.entries = this.entries.filter((entry) => entry.id !== id);
		try {
			await historyDelete(id);
		} catch (error) {
			this.error = String(error);
		}
	}

	async clearFor(requestId: string): Promise<void> {
		this.entries = this.entries.filter((entry) => entry.requestId !== requestId);
		delete this.#selected[requestId];
		try {
			await historyClearRequest(requestId);
		} catch (error) {
			this.error = String(error);
		}
	}

	async clear(): Promise<void> {
		this.entries = [];
		this.#selected = {};
		try {
			await historyClearAll();
		} catch (error) {
			this.error = String(error);
		}
	}
}

export const history = new History();
