import {
	deleteSection,
	endpointKey,
	LOOSE_SECTION_ID,
	listSections,
	loaderCache,
	normalizeBaseUrl,
	runLoader,
	saveSection,
	type LoadedEndpoint,
	type LoaderCache,
	type LoaderRun,
	type SavedRequest,
	type Section
} from './api';

const SAVE_DEBOUNCE_MS = 400;

/**
 * The name a request is born with. Also the marker for "nobody has named this
 * yet", which is what lets the name follow the path until someone does.
 */
export const NEW_REQUEST_NAME = 'New request';

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

	/**
	 * Requests whose name is still following their path, by id.
	 *
	 * Not written to disk, and it doesn't need to be: a following name always
	 * equals its own path once typing stops, so #adopt rebuilds the set from the
	 * section files on load. That keeps the TOML format — which the MCP server
	 * and anyone reading a collection by hand also see — unchanged.
	 */
	#following = new Set<string>();

	/** Last loader run per section id, mirrored from disk. */
	loaderCaches = $state<Record<string, LoaderCache>>({});
	/** Sections whose loader is currently running. */
	loading = $state<Record<string, boolean>>({});

	get selected(): Selection | null {
		return this.findRequest(this.selectedRequestId);
	}

	/**
	 * The request an id names, if one still exists.
	 *
	 * History entries outlive their requests — a request can be deleted, and
	 * some entries never had one, having come from a loader or the MCP server.
	 */
	findRequest(id: string | null): Selection | null {
		if (!id) return null;
		for (const section of this.sections) {
			const request =
				section.requests.find((candidate) => candidate.id === id) ??
				section.overlay.find((candidate) => candidate.id === id);
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

	/** Selects a loaded endpoint from the sidebar. */
	selectLoaded(section: Section, row: LoadedRow): void {
		this.select({ section, request: row.request });
	}

	/**
	 * Selects any request, wherever it was found — sidebar, ⌘K, anywhere.
	 *
	 * A loaded endpoint the user has never opened has no overlay entry yet, so
	 * it's promoted into one first, so its id resolves and edits have somewhere
	 * to live. Promotion is on selection, not on load — otherwise opening a
	 * section would write an entry for every endpoint it reports. A loader only
	 * fills a collection; once filled, its endpoints open like any other.
	 */
	select(selection: Selection): void {
		const { section, request } = selection;
		const known =
			section.requests.some((entry) => entry.id === request.id) ||
			section.overlay.some((entry) => entry.id === request.id);
		if (!known) {
			section.overlay.push({ ...request });
			this.flush(section);
		}
		this.selectedRequestId = request.id;
	}

	/** Requests belonging to no collection, if any have been made. */
	get looseSection(): Section | undefined {
		return this.sections.find((section) => section.id === LOOSE_SECTION_ID);
	}

	/** Everything the sidebar shows as a collection — i.e. not the loose ones. */
	get collectionSections(): Section[] {
		return this.sections.filter((section) => section.id !== LOOSE_SECTION_ID);
	}

	/**
	 * A blank request outside any collection. Creates the reserved section on
	 * first use, so an app that never needs one never grows the file.
	 */
	async createLooseRequest(): Promise<SavedRequest> {
		// No base URL to hang a path off, so it starts empty and takes a full URL.
		return this.createRequest(this.ensureLooseSection(), NEW_REQUEST_NAME, '');
	}

	/** The reserved section, created if this is the first thing to need it. */
	ensureLooseSection(): Section {
		const existing = this.looseSection;
		if (existing) return existing;

		const section: Section = {
			id: LOOSE_SECTION_ID,
			name: 'Loose requests',
			baseUrl: '',
			collapsed: false,
			order: -1,
			auth: { kind: 'none' },
			mcp: { enabled: false, allowWrites: false },
			requests: [],
			overlay: []
		};
		this.sections = [...this.sections, section];
		return section;
	}

	/**
	 * Moves a request within its collection, or into another one.
	 *
	 * Carries the request itself rather than copying fields, so its id survives
	 * — which is what keeps its history and its place in the response pane
	 * attached to it across the move.
	 */
	async moveRequest(
		from: { sectionId: string; requestId: string },
		to: { sectionId: string; requestId?: string; edge?: 'top' | 'bottom' }
	): Promise<void> {
		const source = this.sections.find((section) => section.id === from.sectionId);
		// Moving something out of every collection is what creates the loose
		// section, if nothing has needed it yet.
		const target =
			to.sectionId === LOOSE_SECTION_ID
				? this.ensureLooseSection()
				: this.sections.find((section) => section.id === to.sectionId);
		if (!source || !target) return;

		const at = source.requests.findIndex((request) => request.id === from.requestId);
		if (at < 0) return;
		const [moved] = source.requests.splice(at, 1);

		let index = target.requests.length;
		if (to.requestId) {
			const anchor = target.requests.findIndex((request) => request.id === to.requestId);
			if (anchor >= 0) index = to.edge === 'bottom' ? anchor + 1 : anchor;
		}
		target.requests.splice(index, 0, moved);

		await this.flush(target);
		if (source.id !== target.id) await this.flush(source);
	}

	/** Reorders collections, renumbering so the order survives a restart. */
	async reorderSections(movedId: string, targetId: string, edge: 'top' | 'bottom'): Promise<void> {
		const ordered = this.collectionSections;
		const from = ordered.findIndex((section) => section.id === movedId);
		const anchor = ordered.findIndex((section) => section.id === targetId);
		if (from < 0 || anchor < 0) return;

		const [moved] = ordered.splice(from, 1);
		// The anchor shifts when the moved item came from above it.
		const adjusted = anchor - (from < anchor ? 1 : 0);
		ordered.splice(edge === 'bottom' ? adjusted + 1 : adjusted, 0, moved);

		// Renumber every one: sparse numbering would drift after enough moves.
		await Promise.all(
			ordered.map((section, index) => {
				if (section.order === index) return Promise.resolve();
				section.order = index;
				return this.flush(section);
			})
		);
		// Re-sort in place so the sidebar matches what was just written.
		this.sections = [...this.sections].sort((a, b) => a.order - b.order);
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

	/**
	 * Re-runs loaders whose cache has aged past their TTL.
	 *
	 * Deliberately fire-and-forget and never awaited by startup: the cached
	 * endpoints are already on screen, so a slow or unreachable API delays
	 * nothing. A TTL of 0 means "only when asked".
	 */
	refreshStale(): void {
		const now = Date.now();
		for (const section of this.sections) {
			const loader = section.loader;
			if (!loader?.enabled || loader.ttlSeconds <= 0) continue;

			const cache = this.loaderCaches[section.id];
			const age = now - (cache?.loadedAt ?? 0);
			if (age > loader.ttlSeconds * 1000) this.refresh(section);
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
				this.#adopt(section);
			}
			this.error = null;
			await this.loadCaches();
		} catch (error) {
			this.error = String(error);
		} finally {
			this.loaded = true;
		}
	}

	/**
	 * Works out, from a freshly loaded section, which names were following a
	 * path. A name is taken to be following if it is still the placeholder, or
	 * if it is exactly the path — which is the state this leaves them in.
	 *
	 * The one thing it gets wrong is a request deliberately renamed to its own
	 * path, which it will carry on following. That is the same name either way,
	 * so nobody can tell.
	 */
	#adopt(section: Section): void {
		for (const request of section.requests) {
			if (request.name === NEW_REQUEST_NAME || request.name === request.path) {
				this.#following.add(request.id);
			}
		}
	}

	/**
	 * Keeps an unnamed request's name on its path. Called as the URL is typed.
	 *
	 * Stops for good the moment someone renames the request — see `rename`. An
	 * empty path falls back to the placeholder rather than leaving a nameless row
	 * in the sidebar.
	 */
	followPath(request: SavedRequest): void {
		if (!this.#following.has(request.id)) return;
		const path = request.path.trim();
		// A bare "/" is the path a new request is born with, and an empty one is
		// what a loose request starts from. Neither is something anybody typed,
		// so both keep the placeholder.
		const name = path && path !== '/' ? path : NEW_REQUEST_NAME;
		if (request.name !== name) request.name = name;
	}

	/** A name typed by hand. From here on it is the user's, and stays put. */
	rename(request: SavedRequest, name: string): void {
		this.#following.delete(request.id);
		request.name = name.trim() || 'Untitled';
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
			// Also here, not just on blur: submitting the dialog with Enter never
			// blurs the field, so that path would otherwise store the slash.
			baseUrl: normalizeBaseUrl(baseUrl),
			collapsed: false,
			order: this.collectionSections.length,
			auth: { kind: 'none' },
			// A new collection is shared with agents read-only by default: visible
			// and callable with GET/HEAD/OPTIONS, but writes stay behind their own
			// switch. Hide it entirely by turning the top switch off in settings.
			mcp: { enabled: true, allowWrites: false },
			requests: [],
			overlay: []
		};
		this.sections = [...this.sections, section];
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

	async createRequest(
		section: Section,
		name = NEW_REQUEST_NAME,
		path = '/'
	): Promise<SavedRequest> {
		const request: SavedRequest = {
			id: crypto.randomUUID(),
			name,
			method: 'GET',
			path,
			body: '',
			headers: []
		};
		// Only a request that arrived with the placeholder follows its path; one
		// created with a name already chosen — from the command palette, say —
		// keeps it.
		if (name === NEW_REQUEST_NAME) this.#following.add(request.id);
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

/**
 * Every request across every section, for search — typed, imported and loaded
 * alike. A loader is just a way to fill a collection, so its endpoints belong in
 * the same search as any other; `rowsFor` merges in the user's overlay data.
 */
export function allRequests(sections: Section[]): Selection[] {
	return sections.flatMap((section) => [
		...section.requests.map((request) => ({ section, request })),
		...collections.rowsFor(section).map((row) => ({ section, request: row.request }))
	]);
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
