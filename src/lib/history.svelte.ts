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
	/**
	 * The collection it was sent from, when it had one.
	 *
	 * A request id is unique only *within* a section, and a loaded endpoint's id
	 * — `METHOD /path` — is identical in every collection describing the same
	 * API. Bucketing on the id alone put staging's and production's replies in
	 * one list, so opening either showed whichever was sent last.
	 *
	 * `null` for a scratch send, and for anything recorded before this was
	 * returned; those match any section rather than disappearing.
	 */
	sectionId: string | null;
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
		sectionId: record.sectionId ?? null,
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
	/**
	 * An entry opened straight from the History tab.
	 *
	 * Without this, clicking an entry whose request has since been deleted — or
	 * which belonged to scratch — resolved to no request, so the response pane
	 * fell back to "send a request to see the response" and the entry you just
	 * clicked went nowhere.
	 */
	viewingId = $state<string | null>(null);

	async load(): Promise<void> {
		try {
			this.entries = (await historyList()).map(fromRecord);
			this.error = null;
		} catch (error) {
			this.error = String(error);
		}
	}

	/**
	 * The bucket a request's entries live in.
	 *
	 * Section first, because a request id is only unique inside one: two
	 * collections describing the same API give every loaded endpoint the same
	 * id, and keying on that alone merged their histories.
	 */
	static #bucket(requestId: string, sectionId: string | null | undefined): string {
		return sectionId ? `${sectionId}\u0000${requestId}` : requestId;
	}

	/**
	 * Newest first.
	 *
	 * An entry with no section belongs to whichever collection asks. It was
	 * written before the section came back from the database, and the request it
	 * names is real — dropping it would look like history had been lost.
	 */
	forRequest(requestId: string, sectionId?: string | null): HistoryEntry[] {
		return this.entries.filter(
			(entry) =>
				entry.requestId === requestId &&
				(!sectionId || !entry.sectionId || entry.sectionId === sectionId)
		);
	}

	/** The entry on screen for a request — an explicit pick, else its newest. */
	selectedFor(requestId: string, sectionId?: string | null): HistoryEntry | undefined {
		const mine = this.forRequest(requestId, sectionId);
		const picked = this.#selected[History.#bucket(requestId, sectionId)];
		return mine.find((entry) => entry.id === picked) ?? mine[0];
	}

	select(requestId: string, entryId: string, sectionId?: string | null): void {
		this.#selected[History.#bucket(requestId, sectionId)] = entryId;
	}

	/** The entry opened from the History tab, if it's still around. */
	get viewing(): HistoryEntry | undefined {
		if (!this.viewingId) return undefined;
		return this.entries.find((entry) => entry.id === this.viewingId);
	}

	/** Picking a request in the sidebar means you're no longer viewing history. */
	stopViewing(): void {
		this.viewingId = null;
	}

	async ensureBody(entry: HistoryEntry): Promise<void> {
		if (entry.bodyLoaded || entry.pending) return;
		// Set first so concurrent calls for the same entry don't both fetch.
		entry.bodyLoaded = true;
		try {
			const body = (await historyBody(entry.id)) ?? '';
			// A `release` may have landed while the fetch was in flight — clicking
			// through requests faster than their bodies load. The flag it cleared
			// says this body is no longer wanted; attaching it anyway would park a
			// possibly-huge string on an entry nobody is looking at.
			if (entry.bodyLoaded) entry.body = body;
			this.error = null;
		} catch (error) {
			entry.bodyLoaded = false;
			this.error = String(error);
		}
	}

	start(entry: Omit<HistoryEntry, 'pending' | 'bodyLoaded'>): void {
		// Sending shows the new response, not whatever history was open.
		this.viewingId = null;
		this.entries.unshift({ ...entry, pending: true, bodyLoaded: false });
		this.select(entry.requestId, entry.id, entry.sectionId);
	}

	/**
	 * Body text arriving while the request is still in flight.
	 *
	 * It goes on the entry rather than into a field of its own because the pane
	 * already renders the entry's body — so a growing body shows up with nothing
	 * else having to know that streaming exists.
	 */
	stream(id: string, text: string): void {
		const entry = this.entries.find((candidate) => candidate.id === id);
		if (!entry?.pending) return;
		entry.body = (entry.body ?? '') + text;
	}

	/** A fresh attempt is starting, so whatever streamed before it is void. */
	restartBody(id: string): void {
		const entry = this.entries.find((candidate) => candidate.id === id);
		if (!entry?.pending) return;
		entry.body = '';
	}

	/** The response is already in hand here, so its body needs no round trip. */
	settle(id: string, result: { response?: ResponseData; error?: string }): void {
		const entry = this.entries.find((candidate) => candidate.id === id);
		if (!entry) return;

		entry.pending = false;
		entry.error = result.error;
		if (result.response) {
			const { body, bodyStreamed, ...meta } = result.response;
			entry.response = meta;
			if (bodyStreamed) {
				// The chunks already on the entry *are* the body. An empty
				// `body` here is deliberate — it was omitted so a large
				// response does not cross the IPC bridge twice. If nothing
				// streamed (the channel died), reload from history.
				if (entry.body === undefined) {
					entry.bodyLoaded = false;
					void this.ensureBody(entry);
				} else {
					entry.bodyLoaded = true;
				}
			} else {
				entry.body = body;
				entry.bodyLoaded = true;
			}
		} else {
			entry.response = undefined;
			entry.bodyLoaded = true;
		}
	}

	/**
	 * A body no longer on screen has no business staying in memory.
	 *
	 * `settle` keeps the full body on the entry so the pane can show it without
	 * a round trip — but bodies run to 32 MB, and an afternoon of requests held
	 * that way is a leak with a delay on it. The body is already persisted on
	 * the Rust side, so dropping it here costs one `historyBody` fetch if the
	 * entry is ever looked at again — the same lazy path the History tab uses.
	 */
	release(id: string): void {
		const entry = this.entries.find((candidate) => candidate.id === id);
		// Never a pending entry: its body is still being streamed onto it.
		if (!entry || entry.pending) return;
		entry.body = undefined;
		entry.bodyLoaded = false;
	}

	/**
	 * Deletes are optimistic — the row vanishes on click — so a failed delete
	 * has to put back what it took, or the UI claims an entry is gone that the
	 * database still holds and will cheerfully show again after a restart.
	 */
	async remove(id: string): Promise<void> {
		const index = this.entries.findIndex((entry) => entry.id === id);
		if (index < 0) return;
		const [removed] = this.entries.splice(index, 1);
		try {
			await historyDelete(id);
		} catch (error) {
			this.entries.splice(Math.min(index, this.entries.length), 0, removed);
			this.error = String(error);
		}
	}

	/**
	 * Clears one request's history, in one collection when it has one.
	 *
	 * Scoped by the same rule `forRequest` reads by, so what disappears is
	 * exactly what was on screen — clearing staging used to take production's
	 * entries with it, since both sat under one key.
	 */
	async clearFor(requestId: string, sectionId?: string | null): Promise<void> {
		const mine = (entry: HistoryEntry) =>
			entry.requestId === requestId &&
			(!sectionId || !entry.sectionId || entry.sectionId === sectionId);

		const removed = this.entries.filter(mine);
		const key = History.#bucket(requestId, sectionId);
		const picked = this.#selected[key];
		this.entries = this.entries.filter((entry) => !mine(entry));
		delete this.#selected[key];
		try {
			await historyClearRequest(requestId, sectionId);
		} catch (error) {
			// Newest-first order survives: the survivors kept theirs, and the
			// removed slice kept its own, so a merge by timestamp restores both.
			this.entries = [...this.entries, ...removed].sort((a, b) => b.at - a.at);
			if (picked !== undefined) this.#selected[key] = picked;
			this.error = String(error);
		}
	}

	async clear(): Promise<void> {
		const entries = this.entries;
		const selected = this.#selected;
		this.entries = [];
		this.#selected = {};
		try {
			await historyClearAll();
		} catch (error) {
			this.entries = entries;
			this.#selected = selected;
			this.error = String(error);
		}
	}
}

export const history = new History();
