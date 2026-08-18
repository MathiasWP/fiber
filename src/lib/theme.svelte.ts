export type ThemeMode = 'system' | 'light' | 'dark';

const STORAGE_KEY = 'fetch:theme';
const MODES: ThemeMode[] = ['system', 'light', 'dark'];

/**
 * The resolved theme is applied to `<html data-theme>`. app.html does the same
 * thing inline before first paint; this keeps it in sync afterwards.
 */
class Theme {
	mode = $state<ThemeMode>('system');
	/** What `system` currently resolves to. */
	systemDark = $state(false);

	get resolved(): 'light' | 'dark' {
		if (this.mode === 'system') return this.systemDark ? 'dark' : 'light';
		return this.mode;
	}

	init(): () => void {
		const stored = localStorage.getItem(STORAGE_KEY) as ThemeMode | null;
		if (stored && MODES.includes(stored)) this.mode = stored;

		const query = matchMedia('(prefers-color-scheme: dark)');
		this.systemDark = query.matches;

		const onChange = (event: MediaQueryListEvent) => (this.systemDark = event.matches);
		query.addEventListener('change', onChange);
		return () => query.removeEventListener('change', onChange);
	}

	set(mode: ThemeMode): void {
		this.mode = mode;
		localStorage.setItem(STORAGE_KEY, mode);
	}

	/** Cycles system → light → dark, for the toolbar button. */
	cycle(): void {
		this.set(MODES[(MODES.indexOf(this.mode) + 1) % MODES.length]);
	}

	apply(): void {
		document.documentElement.dataset.theme = this.resolved;
	}
}

export const theme = new Theme();
