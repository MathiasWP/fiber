import extractorSvelte from '@unocss/extractor-svelte';
import { defineConfig, presetIcons, presetWind4, transformerVariantGroup } from 'unocss';

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
	transformers: [transformerVariantGroup()],
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
		// Bits UI menus — `data-highlighted` is what it sets on the active item.
		'menu-content':
			'z-50 min-w-44 rounded-md border border-border bg-panel p-1 shadow-xl outline-none',
		'menu-item':
			'flex items-center gap-2 w-full px-2 py-1.5 rounded text-xs text-text cursor-pointer outline-none select-none data-[highlighted]:bg-raised',
		'menu-item-bad':
			'menu-item text-bad data-[highlighted]:(bg-bad/10 text-bad)',
		'menu-separator': 'my-1 h-px bg-border'
	}
});
