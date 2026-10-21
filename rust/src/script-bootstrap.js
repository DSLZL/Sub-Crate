// Language operations used by native algorithms. These retain actual JS values.
Object.assign(globalThis, SubStoreScriptUtilities);
globalThis.__language = {
  get: (value, key) => value[key],
  optional: (value, key) => value?.[key],
  set: (value, key, item) => { value[key] = item; },
  remove: (value, key) => { delete value[key]; },
  own: (value, key) => Object.hasOwn(value, key),
  text: value => String(value),
  typeOf: value => typeof value,
  truthy: value => !!value,
  equal: (a, b) => a === b,
  looseEqual: (a, b) => a == b,
  gt: (a, b) => a > b,
  lt: (a, b) => a < b,
  le: (a, b) => a <= b,
  add: (a, b) => a + b,
  increment: (value, key) => value[key]++,
  length: value => value.length,
  method: (value, key, args) => Reflect.apply(value[key], value, args),
  starts: (value, prefix) => value.startsWith(prefix),
  regex: (pattern, flags) => new RegExp(pattern, flags),
  regexCached: (() => {
    const NativeRegExp = RegExp;
    const descriptor = Object.getOwnPropertyDescriptor, own = Object.hasOwn;
    const match = Symbol.match, originalMatch = descriptor(NativeRegExp.prototype, match).value;
    const cache = new Map(), get = cache.get.bind(cache), set = cache.set.bind(cache), clear = cache.clear.bind(cache);
    let count = 0;
    return (pattern, flags) => {
      const Constructor = RegExp;
      if (Constructor !== NativeRegExp || typeof pattern !== 'string' || pattern.length > 4096 ||
          (flags !== '' && flags !== 'g' && flags !== 'i' && flags !== 'ig')) {
        return new Constructor(pattern, flags);
      }
      const current = descriptor(NativeRegExp.prototype, match);
      if (!current || !own(current, 'value') || current.value !== originalMatch) {
        return new Constructor(pattern, flags);
      }
      const key = flags + ':' + pattern;
      let template = get(key);
      if (!template) {
        template = new NativeRegExp(pattern, flags);
        if (count === 64) { clear(); count = 0; }
        set(key, template); count++;
      }
      // Clone compiled bytecode; callers always receive fresh lastIndex and identity.
      return new NativeRegExp(template);
    };
  })(),
  jsonParse: value => JSON.parse(value),
  assign: (base, value) => Object.assign(base, value),
  iterable: value => [...value],
  random: () => Math.random(),
  now: () => Date.now(),
  dateTime: () => new Date().getTime(),
  number: value => Number(value),
  multiply: (a, b) => a * b,
  divide: (a, b) => a / b,
  subtract: (a, b) => a - b,
  construct: (constructor, args) => Reflect.construct(constructor, args),
  entries: value => Object.entries(value),
  keysIn: value => { const keys = []; for (const key in value) keys.push(key); return keys; },
  units: value => {
    const units = [];
    for (let index = 0; index < value.length; index++) units.push(value.charCodeAt(index));
    return units;
  },
  spread: (base, value) => ({...base, ...value}),
  except: (value, excluded) => Object.fromEntries(Reflect.ownKeys(Object(value))
    .filter(key => !excluded.includes(key) && Object.prototype.propertyIsEnumerable.call(value, key))
    .map(key => [key, value[key]])),
  resolve: value => Promise.resolve(value),
  sameType: (a, b) => typeof a === typeof b && (a === null) === (b === null),
  isError: value => value instanceof Error,
};
globalThis.console = Object.fromEntries(['log', 'info', 'warn', 'error', 'debug'].map(
  level => [level, (...values) => __hostLog(level, values)],
));
