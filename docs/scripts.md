# Scripts

Sub-Crate runs Sub-Store scripts in an embedded JavaScript engine ([QuickJS](https://github.com/DelSkayn/rquickjs)) instead of Node.js. Most scripts written for Sub-Store work unchanged.

## What works

- Operators, filters and response transforms written in standard JavaScript, including `async` code.
- The arguments Sub-Store passes to scripts: `$arguments`, `$options`, `$substore`, `lodash`, `ProxyUtils`, `yaml`, `Buffer`, `b64d`, `b64e`, `scriptResourceCache`, `flowUtils` and `produceArtifact`.
- Timers, and the storage, cache, HTTP, DNS, MMDB, Gist, age, download, flow and conversion helpers.
- Linked scripts and modules loaded by URL.

## What does not work

- Node.js built-in modules, such as `require('fs')` or `require('node:crypto')`.
- `require`, npm packages and native `.node` addons.
- `process.env` and `$substore.node`.

A script that uses these fails with the usual script error. Sub-Crate never rewrites scripts or falls back to Node.js, and it keeps the saved script text so you can fix it later.

If a script is published in both a JS and a Node version, use the JS version.

## Detecting Sub-Crate

The frontend sees the backend as Sub-Store 2.44.6 so that every feature stays enabled. Scripts receive the real identity in their context:

```js
async function operator(proxies, targetPlatform, context) {
  if (context.env.backend === 'Rust') {
    // Running on Sub-Crate.
  }
  return proxies;
}
```
