import { checkForUpdate, type Update } from './api';

/** Long enough that a machine left running still notices within a day. */
const INTERVAL_MS = 6 * 60 * 60 * 1000;
/** Which version the user has already said no to. */
const DISMISSED_KEY = 'fiber:update-dismissed';

/**
 * Notices new releases; doesn't install them. Fiber is only ad-hoc signed, and
 * a self-replacing app bundle is what Gatekeeper re-examines on next launch —
 * so the toast links to the release page and the download stays deliberate.
 *
 * Every failure here is silent. A background check that can't reach GitHub is
 * not news, and there is nothing the user could do about it anyway.
 */
class Updates {
	available = $state<Update | null>(null);

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
		try {
			const update = await checkForUpdate();
			if (!update) {
				this.available = null;
				return;
			}
			// Don't re-announce a version they've already waved away. A newer one
			// than that still gets through.
			if (localStorage.getItem(DISMISSED_KEY) === update.version) return;
			this.available = update;
		} catch {
			// Offline, rate-limited, GitHub down — all the same non-event.
		}
	}

	/** Hides this version until there's a newer one. */
	dismiss(): void {
		if (this.available) localStorage.setItem(DISMISSED_KEY, this.available.version);
		this.available = null;
	}
}

export const updates = new Updates();
