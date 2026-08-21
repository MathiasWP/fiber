import {
	deleteSection,
	endpointKey,
	LOOSE_SECTION_ID,
	listSections,
	hasSecret,
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
 * `touch` trusts its callers to only speak up when something changed — the
 * effect that watches the selected section fires on actual edits, and guards
 * the one case where it fires for another reason (the selection moving).
 */
class Collections {
	sections = $state<Section[]>([]);
	selectedRequestId = $state<string | null>(null);
	loaded = $state(false);
	error = $state<string | null>(null);

	#timers = new Map<string, ReturnType<typeof setTimeout>>();

	/**
	 * The corrupt-file report from the last load, if there was one.
	 *
	 * Held separately from `error` because that field is also how save failures
	 * surface, and a save that then *succeeds* clears it — which would quietly
	 * dismiss a warning about files that are still broken on disk. A successful
	 * save falls back to this instead of to nothing.
	 */
	#loadError: string | null = null;

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

	/**
	 * Whether a section that wants a credential actually has one, by id.
	 *
	 * Only ever says stored or not stored. Whether the credential still *works*
	 * is not knowable without sending something — an expired cookie is present
	 * and correct right up until the server disagrees — so the sidebar claims
	 * the thing that can be checked and leaves the rest to the response.
	 *
	 * Cheap to ask now: `has_secret` reads the keychain item's attributes rather
	 * than its data, so it needs no authorization and raises no prompt.
	 */
	credential = $state<Record<string, boolean>>({});

	/** Re-checks one section, after its auth or its secret has changed. */
	async refreshCredential(section: Section): Promise<void> {
		const reference = 'secretRef' in section.auth ? section.auth.secretRef : null;
		if (!reference) {
			delete this.credential[section.id];
			return;
		}
		this.credential[section.id] = await hasSecret(reference);
	}

	async refreshCredentials(): Promise<void> {
		await Promise.all(this.sections.map((section) => this.refreshCredential(section)));
	}

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

		// Indexed rather than searched. `find` per endpoint made this quadratic —
		// a manifest of several hundred endpoints against an overlay that grows
		// every time you open one, walked afresh on every render.
		const saved = new Map(section.overlay.map((entry) => [entry.id, entry]));

		const rows: LoadedRow[] = cache.endpoints.map((endpoint: LoadedEndpoint) => {
			const id = endpointKey(endpoint.method, endpoint.path);
			const held = saved.get(id);
			return {
				request: {
					id,
					name: endpoint.name || endpoint.path,
					method: endpoint.method,
					path: endpoint.path,
					// The manifest's body, never the overlay's. The overlay entry is
					// what the editor is actually handed once an endpoint is opened —
					// see `select` — so the merged row's body only matters twice: at
					// promotion, where the manifest body is the right starting point,
					// and at adoption, which only happens when the saved body is empty
					// and the manifest body would have won the merge anyway. Reading
					// `held.body` here did nothing but subscribe every sidebar row to
					// every keystroke typed into a loaded endpoint's body.
					body: endpoint.body || '',
					headers: held?.headers ?? []
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
	 * The body the loader generated for an endpoint — what it looked like
	 * before anybody typed, placeholders and all. Null for ordinary requests,
	 * and for endpoints whose manifest never carried a body.
	 */
	manifestBodyFor(selection: Selection): string | null {
		const cache = this.loaderCaches[selection.section.id];
		if (!cache) return null;
		const endpoint = cache.endpoints.find(
			(candidate) => endpointKey(candidate.method, candidate.path) === selection.request.id
		);
		return endpoint?.body || null;
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
		const held =
			section.requests.find((entry) => entry.id === request.id) ??
			section.overlay.find((entry) => entry.id === request.id);

		// An endpoint opened before its manifest carried a body has an overlay
		// entry holding an empty one, and that entry — not the merged row — is
		// what the editor is handed. So it adopts the body here, at the moment
		// you open it. Only when it has none: anything you have written is
		// yours, and a refresh must not overwrite it.
		if (held && !held.body && request.body) {
			held.body = request.body;
			this.touch(section);
		}

		if (!held) {
			section.overlay.push({ ...request });
			// Debounced like every other edit, not written on the spot. Opening
			// three endpoints in a row used to be three full section writes —
			// stringify, snapshot, TOML, disk — and the entry being written holds
			// nothing of yours yet, so there is nothing to lose by coalescing
			// them. It would simply be re-promoted next time you clicked it.
			this.touch(section);
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
			// Already running: a second pass while the first is in flight would
			// double the requests and race over the same cache.
			if (this.loading[section.id]) continue;

			const cache = this.loaderCaches[section.id];
			const age = now - (cache?.loadedAt ?? 0);
			if (age > loader.ttlSeconds * 1000) this.refresh(section);
		}
	}

	/**
	 * Re-checks staleness whenever the window comes back to the front.
	 *
	 * Startup was the only trigger before, which for an app you leave open for
	 * days meant a TTL almost never came round — coming back to Fiber after an
	 * afternoon of deploys showed yesterday's endpoints. Focus is the moment you
	 * are about to look, so it is the moment worth being current.
	 *
	 * It reuses the staleness rule rather than refetching outright: a TTL of 0
	 * still means "only when asked", which some APIs need it to.
	 */
	watchFocus(): () => void {
		const onFocus = () => this.refreshStale();
		window.addEventListener('focus', onFocus);
		return () => window.removeEventListener('focus', onFocus);
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
			const { sections, errors } = await listSections();
			this.sections = sections;
			for (const section of this.sections) {
				this.#adopt(section);
			}
			// A file that couldn't be read is skipped, not deleted — and saying so
			// is the difference between "my collection is corrupt" and "my
			// collection is gone". The good sections still load around it.
			if (errors.length > 0) {
				const listed = errors.map((entry) => `${entry.file} (${entry.message})`).join(', ');
				const one = errors.length === 1;
				this.#loadError =
					`Skipped ${errors.length} collection ${one ? 'file' : 'files'} that couldn't be read: ${listed}. ` +
					`The ${one ? 'file is' : 'files are'} untouched on disk — fix or remove ${one ? 'it' : 'them'}.`;
			} else {
				this.#loadError = null;
			}
			this.error = this.#loadError;
			await this.loadCaches();
			await this.refreshCredentials();
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

	/**
	 * Schedules a debounced write. Callers are believed: every path that calls
	 * this has just changed something, or fired precisely because something
	 * changed — the effect in the page that watches the selected section only
	 * runs when a field it read actually moved. It used to verify that with a
	 * full JSON.stringify compare, which against a section holding a 30 MB body
	 * was a serialization per keystroke to answer a question the reactivity
	 * system had already answered.
	 */
	touch(section: Section): void {
		clearTimeout(this.#timers.get(section.id));
		this.#timers.set(
			section.id,
			setTimeout(() => {
				this.#timers.delete(section.id);
				this.#write(section);
			}, SAVE_DEBOUNCE_MS)
		);
	}

	/** Whether any section has an edit waiting on its debounce timer. */
	get pending(): boolean {
		return this.#timers.size > 0;
	}

	/**
	 * Runs every pending save now, timers be damned.
	 *
	 * This is the quit path: the debounce exists to spare the disk while you
	 * type, and the one moment it can cost you data is when the app goes away
	 * before a timer fires. Settles rather than rejects — with the window on
	 * its way out there is nobody left to retry, so every write that can land
	 * should, whatever its neighbours did.
	 */
	async flushAll(): Promise<void> {
		const waiting = [...this.#timers.keys()];
		for (const timer of this.#timers.values()) clearTimeout(timer);
		this.#timers.clear();

		await Promise.allSettled(
			waiting.map((id) => {
				const section = this.sections.find((candidate) => candidate.id === id);
				return section ? this.#write(section) : Promise.resolve();
			})
		);
	}

	/** Skips the debounce — for structural changes the user expects to stick. */
	async flush(section: Section): Promise<void> {
		clearTimeout(this.#timers.get(section.id));
		this.#timers.delete(section.id);
		await this.#write(section);
	}

	async #write(section: Section): Promise<void> {
		const snapshot = $state.snapshot(section) as Section;
		// The header table keeps a trailing blank row for editing; it has no
		// business in a file someone might read or diff.
		for (const request of [...snapshot.requests, ...snapshot.overlay]) {
			request.headers = request.headers.filter((header) => header.name.trim().length > 0);
		}

		try {
			await saveSection(snapshot);
			// Back to the standing load warning, if there is one — a save that
			// worked says nothing about the files that never loaded.
			this.error = this.#loadError;
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
		// Optimistic: the row disappears the moment you ask. But everything
		// removed is held on to, because if the delete then fails the file is
		// still on disk — and a sidebar that disagrees with the disk it claims to
		// mirror is worse than a row that briefly came back.
		const index = this.sections.findIndex((candidate) => candidate.id === section.id);
		const selected = this.selectedRequestId;

		this.sections = this.sections.filter((candidate) => candidate.id !== section.id);
		if (section.requests.some((r) => r.id === this.selectedRequestId)) {
			this.selectedRequestId = null;
		}
		clearTimeout(this.#timers.get(section.id));
		this.#timers.delete(section.id);
		try {
			await deleteSection(section.id);
		} catch (error) {
			const restored = [...this.sections];
			restored.splice(Math.max(0, index), 0, section);
			this.sections = restored;
			this.selectedRequestId = selected;
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


