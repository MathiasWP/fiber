export type ThemeMode = 'system' | 'light' | 'dark';

const STORAGE_KEY = 'fiber:theme';
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

	/**
	 * Flips between light and dark. `system` is the starting point rather than a
	 * third stop on the cycle — following the OS is the default until you say
	 * otherwise, and "otherwise" only has two answers.
	 */
	toggle(): void {
		this.set(this.resolved === 'dark' ? 'light' : 'dark');
	}

	/** Back to following the OS. */
	followSystem(): void {
		this.set('system');
	}

	apply(): void {
		document.documentElement.dataset.theme = this.resolved;
	}
}

export const theme = new Theme();
