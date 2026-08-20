import { relaunch } from '@tauri-apps/plugin-process';
import { check, type Update } from '@tauri-apps/plugin-updater';

/** Long enough that a machine left running still notices within a day. */
const INTERVAL_MS = 6 * 60 * 60 * 1000;
/** Which version the user has already said no to. */
const DISMISSED_KEY = 'fiber:update-dismissed';

export type UpdateStage =
	| 'idle'
	| 'available'
	| 'downloading'
	| 'installing'
	/** Swapped in on disk, waiting for the next launch to take effect. */
	| 'staged'
	| 'failed';

/**
 * Finds new releases and installs them.
 *
 * Tauri's updater does the work: it fetches `latest.json` from the GitHub
 * release, checks the bundle against the public key baked into this build,
 * swaps the app in place and then restarts into it. The signature check is the
 * point — nothing unsigned by the release workflow's private key will install,
 * so a hijacked endpoint cannot ship anyone a binary.
 *
 * Failures are quiet up to the moment the user asks for the update, and loud
 * after: a background check that can't reach GitHub is not news, but a download
 * that dies halfway is.
 */
class Updates {
	stage = $state<UpdateStage>('idle');
	version = $state('');
	/** 0–1 while downloading, or null when the server sent no length. */
	progress = $state<number | null>(null);
	error = $state<string | null>(null);

	#update: Update | null = null;
	#downloaded = 0;
	#total = 0;

	/** Starts a check now and every few hours. Returns the teardown. */
	watch(): () => void {
		void this.check();
		const timer = setInterval(() => void this.check(), INTERVAL_MS);

		// A laptop that slept through its interval should catch up on waking,
		// which is the common case for "left open since last week".
		const onFocus = () => void this.check();
		window.addEventListener('focus', onFocus);

		return () => {
			clearInterval(timer);
			window.removeEventListener('focus', onFocus);
		};
	}

	async check(): Promise<void> {
		// Never interrupt a download in flight to go looking for another one.
		if (this.stage !== 'idle') return;

		try {
			const update = await check();
			if (!update) return;
			// Don't re-announce a version they've already waved away. A newer one
			// than that still gets through.
			if (localStorage.getItem(DISMISSED_KEY) === update.version) return;

			this.#update = update;
			this.version = update.version;
			this.stage = 'available';
		} catch {
			// Offline, rate-limited, no release published yet — all the same
			// non-event, and nothing the user could act on.
		}
	}

	/**
	 * Downloads and installs. Restarts into it unless told otherwise.
	 *
	 * `restart: false` is the whole "next time I open it" option: installing
	 * replaces the bundle on disk, but the running process keeps the code it
	 * already loaded, so the new version simply arrives when you next launch.
	 * Nothing is deferred or queued — the work is done either way.
	 */
	async install({ restart = true }: { restart?: boolean } = {}): Promise<void> {
		if (!this.#update || this.stage !== 'available') return;

		this.stage = 'downloading';
		this.progress = null;
		this.error = null;
		this.#downloaded = 0;
		this.#total = 0;

		try {
			await this.#update.downloadAndInstall((event) => {
				if (event.event === 'Started') {
					this.#total = event.data.contentLength ?? 0;
				} else if (event.event === 'Progress') {
					this.#downloaded += event.data.chunkLength;
					// A missing content-length means an indeterminate bar rather than
					// a wrong one.
					this.progress = this.#total > 0 ? this.#downloaded / this.#total : null;
				} else if (event.event === 'Finished') {
					this.progress = 1;
					this.stage = 'installing';
				}
			});

			if (!restart) {
				// Done, and staying out of the way until the app is next opened.
				//
				// Marked dismissed too: this process still reports the old version,
				// so a later check would find the same update again and offer to
				// install what is already sitting on disk.
				localStorage.setItem(DISMISSED_KEY, this.version);
				this.stage = 'staged';
				return;
			}

			// The new version is on disk; this process is still the old one.
			await relaunch();
		} catch (error) {
			this.stage = 'failed';
			this.error = String(error);
		}
	}

	/** Hides this version until there's a newer one. */
	dismiss(): void {
		if (this.version) localStorage.setItem(DISMISSED_KEY, this.version);
		this.reset();
	}

	/** Back to watching, after a dismissal or a failure. */
	reset(): void {
		this.stage = 'idle';
		this.progress = null;
		this.error = null;
		this.#update = null;
	}
}

export const updates = new Updates();
