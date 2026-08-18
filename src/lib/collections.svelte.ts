import {
	deleteSection,
	listSections,
	saveSection,
	type SavedRequest,
	type Section
} from './api';

const SAVE_DEBOUNCE_MS = 400;

export interface Selection {
	section: Section;
	request: SavedRequest;
}

/**
 * Sections, mirrored from disk.
 *
 * Edits mutate the in-memory section and `touch()` schedules a debounced write.
 * Saves are skipped when nothing actually changed, so the effect that watches
 * the selected section can fire freely without causing redundant disk writes.
 */
class Collections {
	sections = $state<Section[]>([]);
	selectedRequestId = $state<string | null>(null);
	loaded = $state(false);
	error = $state<string | null>(null);

	#timers = new Map<string, ReturnType<typeof setTimeout>>();
	#lastSaved = new Map<string, string>();

	get selected(): Selection | null {
		if (!this.selectedRequestId) return null;
		for (const section of this.sections) {
			const request = section.requests.find((r) => r.id === this.selectedRequestId);
			if (request) return { section, request };
		}
		return null;
	}

	async load(): Promise<void> {
		try {
			this.sections = await listSections();
			for (const section of this.sections) {
				this.#lastSaved.set(section.id, JSON.stringify(section));
			}
			this.error = null;
		} catch (error) {
			this.error = String(error);
		} finally {
			this.loaded = true;
		}
	}

	/** Schedules a write if `section` differs from what's on disk. */
	touch(section: Section): void {
		const serialized = JSON.stringify(section);
		if (this.#lastSaved.get(section.id) === serialized) return;

		clearTimeout(this.#timers.get(section.id));
		this.#timers.set(
			section.id,
			setTimeout(() => {
				this.#timers.delete(section.id);
				this.#write(section);
			}, SAVE_DEBOUNCE_MS)
		);
	}

	/** Skips the debounce — for structural changes the user expects to stick. */
	async flush(section: Section): Promise<void> {
		clearTimeout(this.#timers.get(section.id));
		this.#timers.delete(section.id);
		await this.#write(section);
	}

	async #write(section: Section): Promise<void> {
		const serialized = JSON.stringify(section);
		const snapshot = $state.snapshot(section) as Section;
		// The header table keeps a trailing blank row for editing; it has no
		// business in a file someone might read or diff.
		for (const request of snapshot.requests) {
			request.headers = request.headers.filter((header) => header.name.trim().length > 0);
		}

		try {
			await saveSection(snapshot);
			this.#lastSaved.set(section.id, serialized);
			this.error = null;
		} catch (error) {
			this.error = String(error);
		}
	}

	async createSection(name: string, baseUrl: string): Promise<Section> {
		const section: Section = {
			id: crypto.randomUUID(),
			name: name.trim() || 'Untitled',
			baseUrl: baseUrl.trim(),
			collapsed: false,
			auth: { kind: 'none' },
			requests: []
		};
		this.sections = [...this.sections, section].sort((a, b) =>
			a.name.toLowerCase().localeCompare(b.name.toLowerCase())
		);
		await this.flush(section);
		return section;
	}

	async removeSection(section: Section): Promise<void> {
		this.sections = this.sections.filter((candidate) => candidate.id !== section.id);
		if (section.requests.some((r) => r.id === this.selectedRequestId)) {
			this.selectedRequestId = null;
		}
		clearTimeout(this.#timers.get(section.id));
		this.#timers.delete(section.id);
		this.#lastSaved.delete(section.id);
		try {
			await deleteSection(section.id);
		} catch (error) {
			this.error = String(error);
		}
	}

	async createRequest(section: Section, name = 'New request'): Promise<SavedRequest> {
		const request: SavedRequest = {
			id: crypto.randomUUID(),
			name,
			method: 'GET',
			path: '/',
			body: '',
			headers: []
		};
		section.requests.push(request);
		section.collapsed = false;
		this.selectedRequestId = request.id;
		await this.flush(section);
		return request;
	}

	async removeRequest(section: Section, request: SavedRequest): Promise<void> {
		section.requests = section.requests.filter((candidate) => candidate.id !== request.id);
		if (this.selectedRequestId === request.id) this.selectedRequestId = null;
		await this.flush(section);
	}

	async duplicateRequest(section: Section, request: SavedRequest): Promise<void> {
		const copy: SavedRequest = {
			...$state.snapshot(request),
			id: crypto.randomUUID(),
			name: `${request.name} copy`
		};
		const at = section.requests.findIndex((candidate) => candidate.id === request.id);
		section.requests.splice(at + 1, 0, copy);
		this.selectedRequestId = copy.id;
		await this.flush(section);
	}
}

export const collections = new Collections();

/** Every request across every section, for search. */
export function allRequests(sections: Section[]): Selection[] {
	return sections.flatMap((section) =>
		section.requests.map((request) => ({ section, request }))
	);
}

/**
 * Subsequence match, the way command palettes behave: "ugt" finds "user get".
 * Returns null when it doesn't match, otherwise a score where lower is better.
 */
export function fuzzyScore(haystack: string, needle: string): number | null {
	if (!needle) return 0;

	const target = haystack.toLowerCase();
	const query = needle.toLowerCase();

	let score = 0;
	let from = 0;
	for (const char of query) {
		const at = target.indexOf(char, from);
		if (at === -1) return null;
		// Gaps between matched characters make a result less relevant.
		score += at - from;
		from = at + 1;
	}
	// Prefer shorter targets when scores are otherwise equal.
	return score * 100 + target.length;
}
