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
};
globalThis.console = Object.fromEntries(['log', 'info', 'warn', 'error', 'debug'].map(
  level => [level, (...values) => __hostLog(level, values.map(value => String(value)).join(' '))],
));
