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
	splitQuery,
	withQuery,
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

/** A loader run that failed, named and — when it can be — actionable. */
export interface LoaderFailure {
	sectionId: string;
	sectionName: string;
	message: string;
	/**
	 * The API rejected the credential rather than the request, and this section
	 * signs in through a browser — so there is something to click.
	 */
	canSignIn: boolean;
}

/**
 * Whether a loader error reads as "your credential was not accepted".
 *
 * The status arrives inside a message rather than as a field, because the run
 * goes through jq, pagination and OpenAPI enrichment before anything gets to
 * report a status — so the string is what there is. 403 counts alongside 401:
 * plenty of stacks answer a missing or expired session with it, which is
 * exactly the case where the user is left with nothing to act on.
 */
function rejectedCredential(message: string): boolean {
	return /returned 40[13]\b/.test(message);
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
	/**
	 * Which collection the selected request belongs to.
	 *
	 * A request id is only unique *within* a section. A loaded endpoint's id is
	 * `endpointKey` — `"GET /users"` — because that is the identity a saved body
	 * and a refresh have to agree on, and it is deliberately free of the section
	 * so a re-run re-attaches rather than orphaning. The cost is that two
	 * collections describing the same API, staging and production, give every
	 * endpoint the same id in both.
	 *
	 * Selecting by id alone then meant both rows highlighted, `findRequest`
	 * always answered with whichever collection sorted first, and clicking the
	 * other one set the id it already held — so nothing changed and the row
	 * could not be opened at all.
	 */
	selectedSectionId = $state<string | null>(null);
	loaded = $state(false);
	error = $state<string | null>(null);

	/**
	 * The last loader run that failed, if it still matters.
	 *
	 * Separate from `error`, and structured, because a loader failure is the one
	 * error here that is both attributable and actionable: it belongs to a
	 * section, and when the API rejected the credential the fix is a button
	 * rather than a paragraph. `error` stays the catch-all for save and load
	 * failures, which are neither.
	 */
	loaderFailure = $state.raw<LoaderFailure | null>(null);

	#timers = new Map<string, ReturnType<typeof setTimeout>>();
	/**
	 * The last disk write queued for each section.
	 *
	 * Writes are serialized per file: overlapping saves used the same temporary
	 * path on the Rust side, and quitting only knew about debounce timers. Keeping
	 * the promise here both prevents those overlaps and lets the quit path await a
	 * write that has already started.
	 */
	#writes = new Map<string, Promise<boolean>>();

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

	/**
	 * Last loader run per section id, mirrored from disk.
	 *
	 * `$state.raw` because each cache is replaced wholesale — never edited in
	 * place — and a deep proxy would wrap every endpoint in a manifest of
	 * hundreds just to hold something the sidebar only ever reads.
	 */
	loaderCaches = $state.raw<Record<string, LoaderCache>>({});
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

	/**
	 * Sections with a sign-in window open, by id.
	 *
	 * A background refresh must not fire at a section you are in the middle of
	 * signing into: the credential is stale by definition until the capture
	 * lands, so the run is guaranteed to fail, and it failed *loudly* — a 401 or
	 * 403 in the sidebar, raised at the exact moment you opened the sign-in
	 * window, describing a state you were already fixing.
	 */
	signingIn = $state<Record<string, boolean>>({});

	beginSignIn(sectionId: string): void {
		this.signingIn[sectionId] = true;
		// The stale failure this sign-in is meant to resolve; holding onto it
		// while the window is open only invites re-reading an obsolete message.
		if (this.loaderFailure?.sectionId === sectionId) this.loaderFailure = null;
	}

	endSignIn(sectionId: string): void {
		delete this.signingIn[sectionId];
	}

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
		return this.findRequest(this.selectedRequestId, this.selectedSectionId);
	}

	/**
	 * The request an id names, if one still exists.
	 *
	 * History entries outlive their requests — a request can be deleted, and
	 * some entries never had one, having come from a loader or the MCP server.
	 *
	 * `sectionId` disambiguates an id two collections both hold, which is every
	 * loaded endpoint when the same API is set up twice. It is a preference
	 * rather than a filter: a history entry recorded before this existed names
	 * no section, and answering nothing for it would lose the request it points
	 * at — so the search falls back to the first match, which is what it always
	 * did.
	 */
	findRequest(id: string | null, sectionId?: string | null): Selection | null {
		if (!id) return null;

		const inSection = (section: Section) =>
			section.requests.find((candidate) => candidate.id === id) ??
			section.overlay.find((candidate) => candidate.id === id);

		if (sectionId) {
			const named = this.sections.find((section) => section.id === sectionId);
			const request = named && inSection(named);
			if (named && request) return { section: named, request };
		}

		for (const section of this.sections) {
			const request = inSection(section);
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
				request: fromEndpoint(endpoint, held),
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
		if (held && !(held.form?.length) && request.form?.length) {
			held.form = request.form.map((field) => ({ ...field }));
			this.touch(section);
		}
		if (held && !held.bodyKind && request.bodyKind && request.bodyKind !== 'json') {
			held.bodyKind = request.bodyKind;
			this.touch(section);
		}
		if (held && !held.description && request.description) {
			held.description = request.description;
			this.touch(section);
		}
		if (held && !held.tag && request.tag) {
			held.tag = request.tag;
			this.touch(section);
		}
		if (held && !(held.pathParams?.length) && request.pathParams?.length) {
			held.pathParams = request.pathParams.map((param) => ({ ...param }));
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
		this.selectedSectionId = section.id;
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

	/**
	 * The reactive copy of a section, after it has been put in `sections`.
	 *
	 * `$state` wraps objects in a proxy on read. Mutating the original plain
	 * object — `section.requests.push(...)` on the value `createSection` used
	 * to return — updates the data but not the proxy's length source, so the
	 * sidebar keeps saying the collection is empty.
	 */
	#live(section: Section): Section {
		return this.sections.find((candidate) => candidate.id === section.id) ?? section;
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
			timeoutMs: 60_000,
			followRedirects: true,
			acceptInvalidCerts: false,
			proxy: '',
			requests: [],
			overlay: []
		};
		this.sections = [...this.sections, section];
		return this.#live(section);
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
		const loaded = await Promise.all(
			this.sections
				.filter((section) => section.loader)
				.map(async (section) => {
					try {
						return [section.id, await loaderCache(section.id)] as const;
					} catch {
						// A missing cache is simply "nothing loaded yet".
						return null;
					}
				})
		);
		const next = { ...this.loaderCaches };
		for (const entry of loaded) {
			if (entry) next[entry[0]] = entry[1];
		}
		this.loaderCaches = next;
	}

	/**
	 * How many loaded rows a collection would show, without building them.
	 *
	 * `rowsFor` allocates a request object per endpoint. A collapsed collection
	 * only needs the number on its header, and rebuilding hundreds of objects
	 * to print that number — on every collections update — is wasted work.
	 */
	loadedCount(section: Section): number {
		const cache = this.loaderCaches[section.id];
		if (!cache) return 0;
		const reported = new Set(
			cache.endpoints.map((endpoint) => endpointKey(endpoint.method, endpoint.path))
		);
		let missing = 0;
		for (const entry of section.overlay) {
			if (!reported.has(entry.id)) missing++;
		}
		return cache.endpoints.length + missing;
	}

	/**
	 * Re-runs loaders whose cache has aged past their TTL.
	 *
	 * Deliberately fire-and-forget and never awaited by startup: the cached
	 * endpoints are already on screen, so a slow or unreachable API delays
	 * nothing. A TTL of 0 means "only when asked".
	 *
	 * Called at startup and whenever the window comes back to the front
	 * (`<svelte:window onfocus>` on the page) — focus is the moment you are
	 * about to look, so it is the moment worth being current. Startup used to
	 * be the only trigger, which for an app you leave open for days meant a TTL
	 * almost never came round.
	 */
	refreshStale(): void {
		const now = Date.now();
		for (const section of this.sections) {
			const loader = section.loader;
			if (!loader?.enabled || loader.ttlSeconds <= 0) continue;
			// Already running: a second pass while the first is in flight would
			// double the requests and race over the same cache.
			if (this.loading[section.id]) continue;
			// Mid-sign-in: the credential is stale until the capture lands, so
			// this run would fail for a reason the user is already fixing. Focus
			// is one of the two triggers, and opening the sign-in window is
			// precisely a moment the main window loses and regains it.
			if (this.signingIn[section.id]) continue;

			const cache = this.loaderCaches[section.id];
			const age = now - (cache?.loadedAt ?? 0);
			if (age > loader.ttlSeconds * 1000) this.refresh(section);
		}
	}

	/** Replaces one section's cache, leaving the others untouched. */
	#setCache(id: string, cache: LoaderCache): void {
		this.loaderCaches = { ...this.loaderCaches, [id]: cache };
	}

	/** Runs a section's loader and refreshes its cache. Never throws. */
	async refresh(section: Section): Promise<LoaderRun | string> {
		this.loading[section.id] = true;
		try {
			const run = await runLoader(section.id);
			this.#setCache(section.id, { loadedAt: run.loadedAt, endpoints: run.endpoints });
			this.error = this.#loadError;
			if (this.loaderFailure?.sectionId === section.id) this.loaderFailure = null;
			return run;
		} catch (error) {
			const message = String(error);
			this.loaderFailure = {
				sectionId: section.id,
				sectionName: section.name,
				message,
				canSignIn: section.auth.kind === 'browser' && rejectedCredential(message)
			};
			return message;
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
				this.#httpDefaults(section);
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

	/** Fields omitted from disk because they match the default still need a value in the UI. */
	#httpDefaults(section: Section): void {
		if (section.timeoutMs == null) section.timeoutMs = 60_000;
		if (section.followRedirects == null) section.followRedirects = true;
		if (section.acceptInvalidCerts == null) section.acceptInvalidCerts = false;
		if (section.proxy == null) section.proxy = '';
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

	/** Whether any section has an edit queued or already being written. */
	get pending(): boolean {
		return this.#timers.size > 0 || this.#writes.size > 0;
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

		const started = waiting.map((id) => {
			const section = this.sections.find((candidate) => candidate.id === id);
			return section ? this.#write(section) : Promise.resolve(true);
		});
		// Include writes whose debounce fired before quit. `started` also covers
		// any new snapshots queued behind one of those active writes.
		await Promise.allSettled([...new Set([...this.#writes.values(), ...started])]);
	}

	/** Skips the debounce — for structural changes the user expects to stick. */
	async flush(section: Section): Promise<boolean> {
		clearTimeout(this.#timers.get(section.id));
		this.#timers.delete(section.id);
		return this.#write(section);
	}

	async #write(section: Section): Promise<boolean> {
		const snapshot = $state.snapshot(section) as Section;
		// The header table keeps a trailing blank row for editing; it has no
		// business in a file someone might read or diff.
		for (const request of [...snapshot.requests, ...snapshot.overlay]) {
			request.headers = request.headers.filter((header) => header.name.trim().length > 0);
			if (request.form) {
				request.form = request.form.filter(
					(field) => field.name.trim().length > 0 || Boolean(field.file?.trim())
				);
			}
			if (request.pathParams) {
				request.pathParams = request.pathParams.filter((param) => param.name.trim().length > 0);
			}
		}

		const prior = this.#writes.get(section.id);
		const write = (prior ?? Promise.resolve(true)).then(async () => {
			try {
				await saveSection(snapshot);
				// Back to the standing load warning, if there is one — a save that
				// worked says nothing about the files that never loaded.
				this.error = this.#loadError;
				return true;
			} catch (error) {
				this.error = String(error);
				return false;
			}
		});
		this.#writes.set(section.id, write);
		try {
			return await write;
		} finally {
			if (this.#writes.get(section.id) === write) this.#writes.delete(section.id);
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
			// MCP exposure is an explicit collection-level decision. Writes have
			// their own switch after sharing is enabled.
			mcp: { enabled: false, allowWrites: false },
			timeoutMs: 60_000,
			followRedirects: true,
			acceptInvalidCerts: false,
			proxy: '',
			requests: [],
			overlay: []
		};
		this.sections = [...this.sections, section];
		const created = this.#live(section);
		await this.flush(created);
		return created;
	}

	async removeSection(section: Section): Promise<void> {
		// Optimistic: the row disappears the moment you ask. But everything
		// removed is held on to, because if the delete then fails the file is
		// still on disk — and a sidebar that disagrees with the disk it claims to
		// mirror is worse than a row that briefly came back.
		const index = this.sections.findIndex((candidate) => candidate.id === section.id);
		const selected = this.selectedRequestId;
		const selectedIn = this.selectedSectionId;

		this.sections = this.sections.filter((candidate) => candidate.id !== section.id);
		// The whole collection is going, so anything selected inside it goes too —
		// by section rather than by request id, which the collection next door may
		// also hold.
		if (this.selectedSectionId === section.id) {
			this.selectedRequestId = null;
			this.selectedSectionId = null;
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
			this.selectedSectionId = selectedIn;
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
			bodyKind: 'json',
			form: [],
			file: '',
			pathParams: [],
			headers: []
		};
		// Only a request that arrived with the placeholder follows its path; one
		// created with a name already chosen — from the command palette, say —
		// keeps it.
		if (name === NEW_REQUEST_NAME) this.#following.add(request.id);
		const target = this.#live(section);
		target.requests.push(request);
		target.collapsed = false;
		this.selectedRequestId = request.id;
		this.selectedSectionId = target.id;
		await this.flush(target);
		return request;
	}

	async removeRequest(section: Section, request: SavedRequest): Promise<void> {
		section.requests = section.requests.filter((candidate) => candidate.id !== request.id);
		if (this.selectedRequestId === request.id && this.selectedSectionId === section.id) {
			this.selectedRequestId = null;
			this.selectedSectionId = null;
		}
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
		this.selectedSectionId = section.id;
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
 * A loaded endpoint as the editor sees it on first open: identity and naming
 * from the manifest, plus any overlay the user has already written.
 *
 * Query and path parameters from the spec are seeded here so opening an
 * operation is enough to have somewhere to type — the path template itself
 * stays `{petId}`, which is the identity a refresh re-attaches to.
 */
function fromEndpoint(endpoint: LoadedEndpoint, held?: SavedRequest): SavedRequest {
	const id = endpointKey(endpoint.method, endpoint.path);
	if (held) {
		return {
			id,
			name: endpoint.name || endpoint.path,
			method: endpoint.method,
			path: held.path || endpoint.path,
			description: held.description || endpoint.description || '',
			tag: held.tag || endpoint.tag || '',
			body: endpoint.body || '',
			bodyKind: held.bodyKind ?? endpoint.bodyKind ?? 'json',
			form: (held.form?.length ? held.form : endpoint.form)?.map((field) => ({ ...field })) ?? [],
			file: held.file ?? '',
			pathParams: held.pathParams?.length
				? held.pathParams.map((param) => ({ ...param }))
				: pathParamsFrom(endpoint.parameters),
			headers: held.headers ?? []
		};
	}

	const seeded = seedParams(endpoint, endpoint.path);
	return {
		id,
		name: endpoint.name || endpoint.path,
		method: endpoint.method,
		path: seeded.path,
		description: endpoint.description || '',
		tag: endpoint.tag || '',
		body: endpoint.body || '',
		bodyKind: endpoint.bodyKind ?? 'json',
		form: endpoint.form?.map((field) => ({ ...field })) ?? [],
		file: '',
		pathParams: seeded.pathParams,
		headers: []
	};
}

function pathParamsFrom(parameters: LoadedEndpoint['parameters']): { name: string; value: string }[] {
	return (parameters ?? [])
		.filter((param) => param.in === 'path')
		.map((param) => ({ name: param.name, value: param.example || '' }));
}

function seedParams(
	endpoint: LoadedEndpoint,
	path: string
): { path: string; pathParams: { name: string; value: string }[] } {
	const parameters = endpoint.parameters ?? [];
	const pathParams = pathParamsFrom(parameters);
	const query = parameters
		.filter((param) => param.in === 'query')
		.map((param) => ({ name: param.name, value: param.example || '' }));
	if (!splitQuery(path).query && query.some((param) => param.name)) {
		path = withQuery(path, query);
	}
	return { path, pathParams };
}
