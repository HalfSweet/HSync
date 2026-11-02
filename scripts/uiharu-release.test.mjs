// @ts-check
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { validateVersion, versionManifest } from './uiharu-release.mjs'

await test('release versions reject malformed and ambiguous prereleases', () => {
  for (const version of ['0.1.0', '1.2.3-rc.1', '0.2.0-alpha.0']) {
    assert.doesNotThrow(() => validateVersion(version))
  }
  for (const version of ['v0.1.0', '01.2.3', '1.2', '1.2.3-rc..1', '1.2.3-01', '1.2.3;echo']) {
    assert.throws(() => validateVersion(version))
  }
})

await test('versioning updates the release family while preserving external dependencies and features', () => {
  const manifest = `[package]
name = "uiharu"
version = "0.1.0"
license = "Apache-2.0"

[dependencies]
uiharu-core = { path = "../uiharu-core", version = "0.1.0" }
uiharu-protocols = { path = "../uiharu-protocols", version = "0.1.0", optional = true }
serde = { version = "1.0", features = ["derive"] }

[dev-dependencies]
uiharu-core = { path = "../uiharu-core", version = "0.1.0", features = ["testing"] }
uiharu-interp = { path = "../uiharu-interp" }
`
  const updated = versionManifest(manifest, '0.2.0-rc.1')
  assert.equal(updated.match(/version = "0.2.0-rc.1"/g)?.length, 4)
  assert.ok(updated.includes('serde = { version = "1.0", features = ["derive"] }'))
  assert.ok(updated.includes('optional = true'))
  assert.ok(updated.includes('features = ["testing"]'))
  assert.ok(updated.includes('license = "Apache-2.0"'))
  assert.ok(updated.includes('uiharu-interp = { path = "../uiharu-interp" }'))
})

await test('an inherited app version cannot be changed through the uiharu versioner', () => {
  assert.throws(() =>
    versionManifest('[package]\nname = "hsync-core"\nversion.workspace = true\n', '0.2.0'),
  )
})
