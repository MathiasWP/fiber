import {
	deleteSection,
	endpointKey,
	listSections,
	loaderCache,
	runLoader,
	saveSection,
	type LoadedEndpoint,
	type LoaderCache,
	type LoaderRun,
	type SavedRequest,
	type Section
} from './api';

const SAVE_DEBOUNCE_MS = 400;

export interface Selection {
	section: Section;
	request: SavedRequest;
}

/**
 * A row in a loader-backed section: the request the UI edits, plus whether the
 * loader still reports it.
 */
export interface LoadedRow {
	request: SavedRequest;
	/** The loader used to report this and no longer does. */
	missing: boolean;
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

	/** Last loader run per section id, mirrored from disk. */
	loaderCaches = $state<Record<string, LoaderCache>>({});
	/** Sections whose loader is currently running. */
	loading = $state<Record<string, boolean>>({});

	get selected(): Selection | null {
		if (!this.selectedRequestId) return null;
		for (const section of this.sections) {
			const request =
				section.requests.find((r) => r.id === this.selectedRequestId) ??
				section.overlay.find((r) => r.id === this.selectedRequestId);
			if (request) return { section, request };
		}
		return null;
	}

	/**
	 * Loaded endpoints merged with the user data hanging off them.
	 *
	 * The loader owns identity and naming; the user owns body and headers. An
	 * endpoint the loader has stopped reporting keeps its overlay and is marked
	 * `missing` rather than being deleted — losing a body to a refresh is the
	 * one thing this model exists to prevent.
	 */
	rowsFor(section: Section): LoadedRow[] {
		const cache = this.loaderCaches[section.id];
		if (!cache) return [];

		const rows: LoadedRow[] = cache.endpoints.map((endpoint: LoadedEndpoint) => {
			const id = endpointKey(endpoint.method, endpoint.path);
			const saved = section.overlay.find((entry) => entry.id === id);
			return {
				request: {
					id,
					name: endpoint.name || endpoint.path,
					method: endpoint.method,
					path: endpoint.path,
					body: saved?.body ?? '',
					headers: saved?.headers ?? []
				},
				missing: false
			};
		});

		const reported = new Set(rows.map((row) => row.request.id));
		for (const entry of section.overlay) {
			if (!reported.has(entry.id)) rows.push({ request: entry, missing: true });
		}
		return rows;
	}

	/**
	 * Promotes a loaded endpoint into a real overlay entry so edits have
	 * somewhere to live. Called on selection, not on load — otherwise opening a
	 * section would write an entry for every endpoint it reports.
	 */
	selectLoaded(section: Section, row: LoadedRow): void {
		if (!section.overlay.some((entry) => entry.id === row.request.id)) {
			section.overlay.push({ ...row.request });
			this.flush(section);
		}
		this.selectedRequestId = row.request.id;
	}

	/** Reads the cached run for every section that has a loader. */
	async loadCaches(): Promise<void> {
		for (const section of this.sections) {
			if (!section.loader) continue;
			try {
				this.loaderCaches[section.id] = await loaderCache(section.id);
			} catch {
				// A missing cache is simply "nothing loaded yet".
			}
		}
	}

	/** Runs a section's loader and refreshes its cache. Never throws. */
	async refresh(section: Section): Promise<LoaderRun | string> {
		this.loading[section.id] = true;
		try {
			const run = await runLoader(section.id);
			this.loaderCaches[section.id] = { loadedAt: run.loadedAt, endpoints: run.endpoints };
			return run;
		} catch (error) {
			return String(error);
		} finally {
			this.loading[section.id] = false;
		}
	}

	async load(): Promise<void> {
		try {
			this.sections = await listSections();
			for (const section of this.sections) {
				this.#lastSaved.set(section.id, JSON.stringify(section));
			}
			this.error = null;
			await this.loadCaches();
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
		for (const request of [...snapshot.requests, ...snapshot.overlay]) {
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
			requests: [],
			overlay: []
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
