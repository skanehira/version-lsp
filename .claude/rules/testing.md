---
paths: src/**/*.rs, tests/**/*.rs
---

# Testing Rules

## Core Principles

### Tests as Specification
Tests should clearly communicate what the system does. Tests are executable documentation.

### Tests as Safety Net
Tests provide confidence that refactoring hasn't broken anything. They should fail only when they should.

### Test Behavior, Not Implementation
Test externally observable behavior, not implementation details.

**Litmus test**: If a test fails, can you explain what broke for the user? If not, it's not testing behavior.

```rust
// Bad: Asserting configuration values (tautology, meaningless as specification)
assert_eq!(config.enabled, true);
assert_eq!(capabilities.open_close, Some(true));

// Good: Testing behavior
// Test that "when a document is opened, the server does XX"
```

**If refactoring breaks your tests, you're testing implementation details.**

## Parameterized Tests

- Use `rstest` crate for parameterized tests
- Convert multiple similar test cases into a single parameterized test
- Use `#[rstest]` with `#[case(...)]` attributes for test parameters

### When to Check for Parameterization

- After writing 2+ tests for the same function
- During REFACTOR phase of TDD

### When to Convert to Parameterized Tests

**Convert when:**
- Test structure is identical (setup → execute → assert pattern)
- Only input values and expected outputs differ
- Multiple tests verify the same behavior with different data

**Do NOT convert when:**
- Test setup logic differs significantly between cases
- Tests verify different behaviors (not just different inputs)
- Each test requires unique assertions or error handling

Example:
```rust
#[rstest]
#[case("input1", "expected1")]
#[case("input2", "expected2")]
fn test_something(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(process(input), expected);
}
```

## Test Organization

- Place unit tests in the same file as the implementation using `#[cfg(test)] mod tests`
- Use integration tests (`tests/`) only for testing multiple modules together
- Keep test names descriptive: `function_name_scenario_expected_behavior`
