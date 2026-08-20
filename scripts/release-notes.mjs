#!/usr/bin/env node
// Pulls one version's section back out of the CHANGELOG.md that changesets
// writes, for the body of the GitHub release. The release workflow runs on
// main after the Version Packages PR has landed, so by then the changelog is
// the record of what is being shipped.
//
//   node scripts/release-notes.mjs 0.2.0

import { existsSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

const version = process.argv[2]
if (!version) {
	console.error('usage: node scripts/release-notes.mjs <x.y.z>')
	process.exit(1)
}

// There is no changelog until the first release lands one, and a release can be
// forced by hand before then — so a missing file falls back rather than throwing.
const path = join(root, 'CHANGELOG.md')
const changelog = existsSync(path) ? readFileSync(path, 'utf8') : ''

// Split on the version headings and take ours. Simpler than one regex spanning
// heading to heading, and it can't run away to the end of the file.
const section = changelog
	.split(/^## /m)
	.slice(1)
	.find((s) => s.split(/\s/)[0] === version)

console.log(section ? section.split('\n').slice(1).join('\n').trim() : `Fiber v${version}.`)
