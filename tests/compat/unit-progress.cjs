const {writeSync} = require('node:fs');

exports.mochaHooks = {
  beforeEach() {
    writeSync(2, `COMPAT_TEST ${JSON.stringify({event: 'start', name: this.currentTest.fullTitle()})}\n`);
  },
  afterEach() {
    writeSync(2, `COMPAT_TEST ${JSON.stringify({event: 'end', name: this.currentTest.fullTitle(), state: this.currentTest.state})}\n`);
  },
};
