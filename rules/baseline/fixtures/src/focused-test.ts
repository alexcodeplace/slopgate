// Focused-test AST canaries.
test.only('focused test canary', () => {});
it.only('focused spec canary', () => {});
fit('focused shorthand canary', () => {});

// Negative canaries: regular tests and comments should not match.
test('regular test', () => {});
it('regular spec', () => {});
describe('regular suite', () => {});
// test.only('commented focused test', () => {});
// fit('commented focused shorthand', () => {});
