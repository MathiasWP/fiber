<script lang="ts">
	import { javascript } from '@codemirror/lang-javascript';
	import { json, jsonParseLinter } from '@codemirror/lang-json';
	import { linter, lintGutter } from '@codemirror/lint';
	import { Compartment, EditorState } from '@codemirror/state';
	import { oneDark } from '@codemirror/theme-one-dark';
	import { EditorView, placeholder as placeholderExt } from '@codemirror/view';
	import { basicSetup } from 'codemirror';
	import { untrack } from 'svelte';
	import { theme } from '$lib/theme.svelte';

	interface Props {
		value?: string;
		readonly?: boolean;
		/** JSON gets highlighting, folding and inline parse errors. */
		language?: 'json' | 'text' | 'typescript';
		placeholder?: string;
	}

	let {
		value = $bindable(''),
		readonly = false,
		language = 'json',
		placeholder = ''
	}: Props = $props();

	let host = $state<HTMLDivElement>();
	let view: EditorView | undefined;

	const editable = new Compartment();
	const languageConf = new Compartment();
	const themeConf = new Compartment();

	const parseJson = jsonParseLinter();

	function languageExtensions(lang: 'json' | 'text' | 'typescript') {
		if (lang === 'typescript') return [javascript({ typescript: true })];
		if (lang !== 'json') return [];
		return [
			json(),
			// An empty document is not a JSON error — it's a body you haven't
			// written yet, or a response with no content. Reporting "Unexpected
			// EOF" there is just noise.
			linter((target) => (target.state.doc.length === 0 ? [] : parseJson(target))),
			lintGutter()
		];
	}

	$effect(() => {
		if (!host) return;

		const instance = new EditorView({
			parent: host,
			state: EditorState.create({
				doc: untrack(() => value),
				extensions: [
					basicSetup,
					themeConf.of(untrack(() => (theme.resolved === 'dark' ? oneDark : []))),
					languageConf.of(languageExtensions(untrack(() => language))),
					editable.of(EditorView.editable.of(!untrack(() => readonly))),
					EditorState.readOnly.of(untrack(() => readonly)),
					EditorView.lineWrapping,
					placeholderExt(untrack(() => placeholder)),
					EditorView.theme({
						'&': { height: '100%', fontSize: '13px' },
						'.cm-scroller': { fontFamily: 'var(--font-mono)' },
						'&.cm-focused': { outline: 'none' }
					}),
					EditorView.updateListener.of((update) => {
						if (!update.docChanged) return;
						value = update.state.doc.toString();
					})
				]
			})
		});

		view = instance;
		return () => {
			instance.destroy();
			view = undefined;
		};
	});

	// Push external changes in without clobbering what the user is typing — the
	// equality check is what stops this ping-ponging with the update listener.
	$effect(() => {
		const next = value;
		if (!view || next === view.state.doc.toString()) return;
		view.dispatch({
			changes: { from: 0, to: view.state.doc.length, insert: next }
		});
	});

	$effect(() => {
		view?.dispatch({
			effects: languageConf.reconfigure(languageExtensions(language))
		});
	});

	$effect(() => {
		view?.dispatch({
			effects: themeConf.reconfigure(theme.resolved === 'dark' ? oneDark : [])
		});
	});

	$effect(() => {
		view?.dispatch({
			effects: editable.reconfigure(EditorView.editable.of(!readonly))
		});
	});

	export function format(): void {
		if (!view) return;
		try {
			const pretty = JSON.stringify(JSON.parse(view.state.doc.toString()), null, 2);
			view.dispatch({
				changes: { from: 0, to: view.state.doc.length, insert: pretty }
			});
		} catch {
			// Not valid JSON — the linter is already saying so in the gutter.
		}
	}
</script>

<div bind:this={host} class="h-full overflow-hidden"></div>
