import extractorSvelte from '@unocss/extractor-svelte';
import { defineConfig, presetIcons, presetWind4 } from 'unocss';

/**
 * Colours are RGB channel triplets in CSS variables (see src/app.css) so a
 * theme swap is just a variable swap.
 *
 * No `%alpha` placeholder here: presetWind4 applies opacity modifiers itself by
 * wrapping the colour in `color-mix()`, and an unsubstituted `%alpha` would make
 * that whole declaration invalid — every colour would silently disappear.
 */
const themed = (name: string) => `rgb(var(--c-${name}))`;

export default defineConfig({
	// Driven by @unocss/postcss rather than the Vite plugin — see postcss.config.mjs
	// for why — so classes are found by scanning the filesystem.
	content: {
		filesystem: ['src/**/*.{html,js,ts,svelte}']
	},
	extractors: [extractorSvelte()],
	presets: [
		presetWind4(),
		presetIcons({
			scale: 1.2,
			extraProperties: { display: 'inline-block', 'vertical-align': 'middle' }
		})
	],
	// No transformers, and none would run: the PostCSS pipeline scans files for
	// class names but never rewrites them, so `hover:(a b)` in markup stays
	// split-by-space junk in the DOM and matches no rule. Write variants out in
	// full — `hover:a hover:b`. Shortcuts below are exempt: their values are
	// expanded config-side, where variant groups do work.
	theme: {
		colors: {
			bg: themed('bg'),
			panel: themed('panel'),
			raised: themed('raised'),
			border: themed('border'),
			muted: themed('muted'),
			text: themed('text'),
			accent: themed('accent'),
			// HTTP status families.
			ok: themed('ok'),
			warn: themed('warn'),
			bad: themed('bad')
		},
		font: {
			mono: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace'
		}
	},
	shortcuts: {
		'input-base':
			'bg-raised border border-border rounded px-2 py-1.5 text-text outline-none focus:border-accent transition-colors',
		'btn-base':
			'inline-flex items-center gap-1.5 rounded px-3 py-1.5 font-medium transition-colors disabled:(opacity-40 cursor-not-allowed)',
		'btn-primary': 'btn-base bg-accent text-white hover:bg-accent/85',
		'btn-ghost': 'btn-base text-muted hover:(bg-raised text-text)',
		/*
		 * The layers, low to high. Everything that floats picks one of these, so
		 * two things that can be open at once can't argue about which is on top:
		 *
		 *   40/50  the settings drawer and its scrim — anchored in a pane, so it
		 *          covers that pane and nothing else
		 *   60     dialogs — portalled to `body`, so they cover the window, which
		 *          means they have to sit above a drawer rather than under it
		 *   70     menus and tooltips — a popover belongs above whatever opened
		 *          it, and it can be opened from either of the two layers above
		 *   100    the update toast
		 *   200    the crash banner, which is the one thing that always shows
		 *
		 * A dialog with no z-index at all is the trap: it lands at `auto` beside
		 * the app root and disappears under the drawer.
		 */
		'dialog-scrim': 'fixed inset-0 z-60 bg-black/50',
		// Bits UI menus — `data-highlighted` is what it sets on the active item.
		'menu-content':
			'z-70 min-w-44 rounded-md border border-border bg-panel p-1 shadow-xl outline-none',
		'menu-item':
			'flex items-center gap-2 w-full px-2 py-1.5 rounded text-xs text-text cursor-default outline-none select-none data-[highlighted]:bg-raised',
		'menu-item-bad':
			'menu-item text-bad data-[highlighted]:(bg-bad/10 text-bad)',
		'menu-separator': 'my-1 h-px bg-border',
		// Same surface as the menus, so a hint and a menu read as one family.
		tooltip:
			'z-70 rounded border border-border bg-panel px-2 py-1 text-2.5 text-text shadow-lg'
	}
});
