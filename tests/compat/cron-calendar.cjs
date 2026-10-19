// Execute only on the Actions runner; native Rust tests assert these same instants.
if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Actions only');
const {createRequire} = require('node:module');
const path = require('node:path');
const backend = createRequire(path.resolve(__dirname, '../../backend/package.json'));
const {CronTime} = backend('cron');
const assert = require('node:assert/strict');
const cases = [
  ['*/15 * * * * *', '2024-07-01T00:00:14.1Z', 'UTC', '2024-07-01T00:00:15Z'],
  ['0 9 * * *', '2024-07-01T08:59:59+08:00', 'Asia/Shanghai', '2024-07-01T01:00:00Z'],
  ['0 0 0 29 feb *', '2025-03-01T00:00:00Z', 'UTC', '2028-02-29T00:00:00Z'],
  ['0 0 0 13 * fri', '2024-07-01T00:00:00Z', 'UTC', '2024-07-05T00:00:00Z'],
  ['0 30 2 * * *', '2024-03-10T01:00:00-05:00', 'America/New_York', '2024-03-10T07:00:00Z'],
  ['0 30 1 * * *', '2024-11-03T01:45:00-04:00', 'America/New_York', '2024-11-04T06:30:00Z'],
  ['0 30 1 * * *', '2024-11-03T01:00:00-05:00', 'America/New_York', '2024-11-03T06:30:00Z'],
];
for (const [expression, start, zone, expected] of cases) {
  const actual = new CronTime(expression).getNextDateFrom(new Date(start), zone).toMillis();
  assert.equal(actual, Date.parse(expected), `${expression} ${start} ${zone}`);
}
console.log(JSON.stringify({status:'passed', cases}));
