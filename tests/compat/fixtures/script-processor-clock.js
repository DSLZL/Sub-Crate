// One deterministic clock for source cache constructors, assertions and reloads.
globalThis.probeNow = 2000000000000;
const ProbeRealDate = Date;
globalThis.Date = class extends ProbeRealDate {
  constructor(...args) { super(...(args.length ? args : [probeNow])); }
  static now() { return probeNow; }
};
