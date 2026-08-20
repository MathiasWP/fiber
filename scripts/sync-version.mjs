#!/usr/bin/env node
// Copies the version out of package.json and into the three places Rust and
// Tauri keep their own copy of it.
//
// Changesets owns the version, but it only knows about package.json — so this
// runs straight after `changeset version` (see the `version` script) and drags
// the rest along. `tauri.conf.json` is the one that matters most: it is the
// version the bundler stamps onto the .dmg/.msi/.deb, so it is the one a user
// can actually see.

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

// Rewritten by regex rather than a parse/serialise round-trip so each file keeps
// its own formatting — and so Cargo.lock, which is not ours to reformat, is
// touched as little as possible.
function sync(relative, pattern, replacement) {
	const path = join(root, relative)
	const before = readFileSync(path, 'utf8')
	const after = before.replace(pattern, replacement)
	if (after === before && !before.includes(`"${version}"`)) {
		throw new Error(`found no version to sync in ${relative}`)
	}
	writeFileSync(path, after)
}

const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))

sync('src-tauri/tauri.conf.json', /"version": "[^"]*"/, `"version": "${version}"`)
// Anchored to the [package] table so a dependency's inline `version = ` is safe.
sync('src-tauri/Cargo.toml', /(\[package\][\s\S]*?\nversion = ")[^"]*(")/, `$1${version}$2`)
// Cargo would fix this itself on the next build, but committing it keeps the
// lockfile honest and keeps the diff in the release PR complete.
sync('src-tauri/Cargo.lock', /(\[\[package\]\]\nname = "fiber"\nversion = ")[^"]*(")/, `$1${version}$2`)

console.log(`synced version ${version} into tauri.conf.json, Cargo.toml and Cargo.lock`)
