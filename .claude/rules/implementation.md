---
paths: src/**/*.rs
---

# Implementation Rules

## Error Handling Patterns

### When returning `Option` (not `Result`)

Use `inspect_err` + `ok()` to log the error before converting to `Option`:

```rust
// Good: Use inspect_err for logging before converting to Option
let Some(value) = fallible_operation()
    .inspect_err(|e| warn!("Operation failed: {}", e))
    .ok()
else {
    return default_value;
};

// Bad: Using match is verbose
let value = match fallible_operation() {
    Ok(v) => v,
    Err(e) => {
        warn!("Operation failed: {}", e);
        return default_value;
    }
};

```

### When returning `Result`

- Use `inspect_err` for logging without changing the error type
- Use `map_err` for error type conversion

```rust
// Good: inspect_err for logging, map_err for conversion
fn process() -> Result<Value, MyError> {
    fallible_operation()
        .inspect_err(|e| warn!("Operation failed: {}", e))
        .map_err(MyError::from)
}

// Good: Just logging, no conversion needed
fn process() -> Result<Value, SameError> {
    fallible_operation()
        .inspect_err(|e| warn!("Operation failed: {}", e))
}
```

## Early Returns with `let-else`

Use `let-else` pattern for early returns instead of nested `if let` or `match`:

```rust
// Good: let-else for early return
let Some(value) = optional_value else {
    return Error::NotFound;
};

// Bad: Nested structure
if let Some(value) = optional_value {
    // ... deep nesting
} else {
    return Error::NotFound;
}
```
